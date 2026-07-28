use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use url::Url;

use crate::VERSION;
use crate::command::safe_environment;
use crate::permission::{
    Capability, PermissionError, PermissionGate, PermissionRequest, RiskLevel,
};
use crate::workspace::{Workspace, WorkspaceError};

const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const REQUEST_TIMEOUT_SECONDS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageServerConfig {
    pub language_id: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Maps a child environment variable to a host environment variable after approval.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum LspError {
    #[error(transparent)]
    Permission(#[from] PermissionError),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error("language server command cannot be empty")]
    EmptyCommand,
    #[error("language-server environment variable is not set: {0}")]
    MissingEnvironment(String),
    #[error("could not start language server: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("language server did not expose stdin or stdout")]
    MissingPipe,
    #[error("LSP transport failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid LSP JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid workspace file URL: {0}")]
    InvalidUrl(String),
    #[error("invalid LSP Content-Length header")]
    InvalidContentLength,
    #[error("LSP message exceeded the {0}-byte limit")]
    MessageTooLarge(usize),
    #[error("language server closed its output")]
    Closed,
    #[error("LSP request failed ({code}): {message}")]
    Rpc { code: i64, message: String },
    #[error("LSP response did not contain a result")]
    MissingResult,
    #[error("language server did not respond within {0} seconds")]
    TimedOut(u64),
}

/// Minimal Language Server Protocol client for symbols and other project intelligence requests.
pub struct LspClient {
    workspace: Arc<Workspace>,
    config: LanguageServerConfig,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl LspClient {
    pub async fn connect(
        workspace: Arc<Workspace>,
        permissions: Arc<PermissionGate>,
        config: LanguageServerConfig,
    ) -> Result<Self, LspError> {
        if config.command.trim().is_empty() {
            return Err(LspError::EmptyCommand);
        }
        permissions.authorize(&PermissionRequest {
            capability: Capability::ExecuteCommand,
            risk: RiskLevel::Medium,
            summary: format!("start {} language server", config.language_id),
            resource: config.command.clone(),
            details: vec![format!("arguments: {}", config.args.join(" "))],
        })?;
        let mut explicit_environment = Vec::new();
        for (target, source) in &config.env {
            permissions.authorize(&PermissionRequest {
                capability: Capability::AccessSecret,
                risk: RiskLevel::High,
                summary: format!("share {source} with {} language server", config.language_id),
                resource: source.clone(),
                details: vec![format!("Available to the server process as {target}")],
            })?;
            let value = std::env::var_os(source)
                .ok_or_else(|| LspError::MissingEnvironment(source.clone()))?;
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
        let mut child = command.spawn().map_err(LspError::Spawn)?;
        let stdin = child.stdin.take().ok_or(LspError::MissingPipe)?;
        let stdout = child.stdout.take().ok_or(LspError::MissingPipe)?;
        let mut client = Self {
            workspace,
            config,
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        };
        let root_uri = client.workspace_uri()?;
        client
            .request(
                "initialize",
                json!({
                    "processId": Value::Null,
                    "rootUri": root_uri,
                    "capabilities": {
                        "textDocument": {"documentSymbol": {"hierarchicalDocumentSymbolSupport": true}}
                    },
                    "clientInfo": {"name": "kernex", "version": VERSION}
                }),
            )
            .await?;
        client.notification("initialized", json!({})).await?;
        Ok(client)
    }

    pub async fn document_symbols(&mut self, path: &str) -> Result<Value, LspError> {
        let resolved = self.workspace.resolve_existing(path)?;
        let text = self.workspace.read_text(&resolved)?;
        let uri = Url::from_file_path(&resolved)
            .map_err(|_| LspError::InvalidUrl(resolved.display().to_string()))?
            .to_string();
        self.notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": self.config.language_id,
                    "version": 1,
                    "text": text,
                }
            }),
        )
        .await?;
        self.request(
            "textDocument/documentSymbol",
            json!({"textDocument": {"uri": uri}}),
        )
        .await
    }

    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, LspError> {
        tokio::time::timeout(
            Duration::from_secs(REQUEST_TIMEOUT_SECONDS),
            self.request_inner(method, params),
        )
        .await
        .map_err(|_| LspError::TimedOut(REQUEST_TIMEOUT_SECONDS))?
    }

    async fn request_inner(&mut self, method: &str, params: Value) -> Result<Value, LspError> {
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
                    return Err(LspError::Rpc {
                        code: error.get("code").and_then(Value::as_i64).unwrap_or(-32603),
                        message: error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown LSP error")
                            .to_owned(),
                    });
                }
                return message
                    .get("result")
                    .cloned()
                    .ok_or(LspError::MissingResult);
            }
            if message.get("method").is_some() && message.get("id").is_some() {
                self.reject_server_request(&message).await?;
            }
        }
    }

    async fn notification(&mut self, method: &str, params: Value) -> Result<(), LspError> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await
    }

    async fn send(&mut self, message: &Value) -> Result<(), LspError> {
        let body = serde_json::to_vec(message)?;
        if body.len() > MAX_MESSAGE_BYTES {
            return Err(LspError::MessageTooLarge(MAX_MESSAGE_BYTES));
        }
        self.stdin
            .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
            .await?;
        self.stdin.write_all(&body).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn receive(&mut self) -> Result<Value, LspError> {
        let mut content_length = None;
        loop {
            let mut header = String::new();
            if self.stdout.read_line(&mut header).await? == 0 {
                return Err(LspError::Closed);
            }
            if header == "\r\n" || header == "\n" {
                break;
            }
            if let Some(value) = header
                .strip_prefix("Content-Length:")
                .or_else(|| header.strip_prefix("content-length:"))
            {
                content_length = value.trim().parse::<usize>().ok();
            }
        }
        let length = content_length.ok_or(LspError::InvalidContentLength)?;
        if length > MAX_MESSAGE_BYTES {
            return Err(LspError::MessageTooLarge(MAX_MESSAGE_BYTES));
        }
        let mut body = vec![0; length];
        self.stdout.read_exact(&mut body).await?;
        Ok(serde_json::from_slice(&body)?)
    }

    async fn reject_server_request(&mut self, message: &Value) -> Result<(), LspError> {
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": message.get("id").cloned().unwrap_or(Value::Null),
            "error": {"code": -32601, "message": "Kernex does not support this server request"}
        }))
        .await
    }

    fn workspace_uri(&self) -> Result<String, LspError> {
        Url::from_directory_path(self.workspace.root())
            .map(|url| url.to_string())
            .map_err(|_| LspError::InvalidUrl(self.workspace.root().display().to_string()))
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}
