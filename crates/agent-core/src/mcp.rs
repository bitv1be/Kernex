use std::collections::{BTreeMap, HashSet};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::VERSION;
use crate::command::safe_environment;
use crate::permission::{
    Capability, PermissionError, PermissionGate, PermissionRequest, RiskLevel,
};
use crate::workspace::Workspace;

const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOOL_PAGES: usize = 100;
const REQUEST_TIMEOUT_SECONDS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Maps a child environment variable to the host environment variable containing its value.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema: Value,
    pub annotations: Option<Value>,
}

#[derive(Debug, Error)]
pub enum McpError {
    #[error(transparent)]
    Permission(#[from] PermissionError),
    #[error("MCP server command cannot be empty")]
    EmptyCommand,
    #[error("MCP credential environment variable is not set: {0}")]
    MissingEnvironment(String),
    #[error("could not start MCP server {server}: {source}")]
    Spawn {
        server: String,
        #[source]
        source: std::io::Error,
    },
    #[error("MCP server {0} did not expose stdin")]
    MissingStdin(String),
    #[error("MCP server {0} did not expose stdout")]
    MissingStdout(String),
    #[error("MCP transport failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("MCP message exceeded the {0}-byte limit")]
    MessageTooLarge(usize),
    #[error("MCP server closed its output")]
    Closed,
    #[error("invalid MCP JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("MCP request failed ({code}): {message}")]
    Rpc { code: i64, message: String },
    #[error("MCP response did not contain a result")]
    MissingResult,
    #[error("MCP server {server} did not respond within {seconds} seconds")]
    TimedOut { server: String, seconds: u64 },
    #[error("MCP server returned a repeated or excessive tools/list cursor")]
    InvalidPagination,
}

/// Permissioned MCP stdio client using newline-delimited JSON-RPC 2.0 messages.
pub struct McpClient {
    config: McpServerConfig,
    permissions: Arc<PermissionGate>,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    server_info: Option<Value>,
}

impl McpClient {
    pub async fn connect(
        workspace: Arc<Workspace>,
        permissions: Arc<PermissionGate>,
        config: McpServerConfig,
    ) -> Result<Self, McpError> {
        if config.command.trim().is_empty() {
            return Err(McpError::EmptyCommand);
        }
        permissions.authorize(&PermissionRequest {
            capability: Capability::ExecuteCommand,
            risk: RiskLevel::High,
            summary: format!("start MCP server {}", config.name),
            resource: config.command.clone(),
            details: vec![format!("arguments: {}", config.args.join(" "))],
        })?;

        let mut explicit_environment = Vec::new();
        for (target, source) in &config.env {
            permissions.authorize(&PermissionRequest {
                capability: Capability::AccessSecret,
                risk: RiskLevel::High,
                summary: format!("share {source} with MCP server {}", config.name),
                resource: source.clone(),
                details: vec![format!("Available to the server process as {target}")],
            })?;
            let value = std::env::var_os(source)
                .ok_or_else(|| McpError::MissingEnvironment(source.clone()))?;
            explicit_environment.push((target.clone(), value));
        }
        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .current_dir(workspace.root())
            .env_clear()
            .envs(safe_environment())
            .envs(explicit_environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|source| McpError::Spawn {
            server: config.name.clone(),
            source,
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::MissingStdin(config.name.clone()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::MissingStdout(config.name.clone()))?;
        let mut client = Self {
            config,
            permissions,
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            server_info: None,
        };
        let initialized = client
            .request(
                "initialize",
                json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {"roots": {"listChanged": false}},
                    "clientInfo": {
                        "name": "kernex",
                        "title": "Kernex",
                        "version": VERSION,
                        "description": "Native provider-independent coding agent"
                    }
                }),
            )
            .await?;
        client.server_info = initialized.get("serverInfo").cloned();
        client
            .notification("notifications/initialized", json!({}))
            .await?;
        Ok(client)
    }

    pub fn name(&self) -> &str {
        &self.config.name
    }

    pub fn server_info(&self) -> Option<&Value> {
        self.server_info.as_ref()
    }

    pub async fn list_tools(&mut self) -> Result<Vec<McpTool>, McpError> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        for _ in 0..MAX_TOOL_PAGES {
            let params = cursor
                .as_ref()
                .map_or_else(|| json!({}), |cursor| json!({"cursor": cursor}));
            let result = self.request("tools/list", params).await?;
            tools.extend(parse_tools(&result)?);
            let next_cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .filter(|cursor| !cursor.is_empty())
                .map(str::to_owned);
            let Some(next_cursor) = next_cursor else {
                return Ok(tools);
            };
            if !seen_cursors.insert(next_cursor.clone()) {
                return Err(McpError::InvalidPagination);
            }
            cursor = Some(next_cursor);
        }
        Err(McpError::InvalidPagination)
    }

    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, McpError> {
        self.permissions.authorize(&PermissionRequest {
            capability: Capability::ExecuteCommand,
            risk: RiskLevel::Medium,
            summary: format!("call MCP tool {}::{name}", self.config.name),
            resource: self.config.name.clone(),
            details: vec![arguments.to_string()],
        })?;
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
            .await
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, McpError> {
        let server = self.config.name.clone();
        tokio::time::timeout(
            Duration::from_secs(REQUEST_TIMEOUT_SECONDS),
            self.request_inner(method, params),
        )
        .await
        .map_err(|_| McpError::TimedOut {
            server,
            seconds: REQUEST_TIMEOUT_SECONDS,
        })?
    }

    async fn request_inner(&mut self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .await?;

        loop {
            let message = self.receive().await?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = message.get("error") {
                    return Err(McpError::Rpc {
                        code: error.get("code").and_then(Value::as_i64).unwrap_or(-32603),
                        message: error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown MCP error")
                            .to_owned(),
                    });
                }
                return message
                    .get("result")
                    .cloned()
                    .ok_or(McpError::MissingResult);
            }
            if message.get("method").is_some() && message.get("id").is_some() {
                self.reject_server_request(&message).await?;
            }
        }
    }

    async fn notification(&mut self, method: &str, params: Value) -> Result<(), McpError> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await
    }

    async fn send(&mut self, message: &Value) -> Result<(), McpError> {
        let encoded = serde_json::to_vec(message)?;
        if encoded.len() > MAX_MESSAGE_BYTES {
            return Err(McpError::MessageTooLarge(MAX_MESSAGE_BYTES));
        }
        self.stdin.write_all(&encoded).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn receive(&mut self) -> Result<Value, McpError> {
        let mut line = String::new();
        let bytes = self.stdout.read_line(&mut line).await?;
        if bytes == 0 {
            return Err(McpError::Closed);
        }
        if bytes > MAX_MESSAGE_BYTES {
            return Err(McpError::MessageTooLarge(MAX_MESSAGE_BYTES));
        }
        Ok(serde_json::from_str(line.trim_end())?)
    }

    async fn reject_server_request(&mut self, message: &Value) -> Result<(), McpError> {
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": message.get("id").cloned().unwrap_or(Value::Null),
            "error": {"code": -32601, "message": "Kernex does not support this server request"}
        }))
        .await
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

fn parse_tools(result: &Value) -> Result<Vec<McpTool>, McpError> {
    result
        .get("tools")
        .and_then(Value::as_array)
        .ok_or(McpError::MissingResult)?
        .iter()
        .map(|tool| {
            Ok(McpTool {
                name: tool
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or(McpError::MissingResult)?
                    .to_owned(),
                title: tool.get("title").and_then(Value::as_str).map(str::to_owned),
                description: tool
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                input_schema: tool
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object"})),
                annotations: tool.get("annotations").cloned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tool_metadata() {
        let tools = parse_tools(&json!({
            "tools": [{
                "name": "lookup",
                "title": "Lookup",
                "description": "Find a value",
                "inputSchema": {"type": "object"},
                "annotations": {"readOnlyHint": true}
            }]
        }))
        .unwrap();
        assert_eq!(tools[0].name, "lookup");
        assert_eq!(tools[0].title.as_deref(), Some("Lookup"));
    }
}
