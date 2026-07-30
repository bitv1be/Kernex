//! Codex App Server JSON-RPC integration.
//!
//! App Server is a complete agent runtime rather than a completion endpoint. This module owns
//! its JSONL-over-stdio lifecycle and exposes account, subscription, model, and turn operations to
//! every Kernex presentation layer.

use std::collections::{BTreeMap, VecDeque};
use std::env;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::agent::{AgentEvent, AgentRunResult, EventSink};
use crate::permission::{
    Approver, Capability, PermissionDecision, PermissionMode, PermissionRequest, RiskLevel,
};
use crate::provider::{Message, ProviderModel, ProviderStreamEvent, Role, TokenUsage, ToolCall};

const MAX_JSON_LINE_BYTES: usize = 32 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);

/// Command used to launch the local Codex App Server process.
#[derive(Debug, Clone)]
pub struct CodexAppServerConfig {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
}

impl Default for CodexAppServerConfig {
    fn default() -> Self {
        let executable = env::var_os("KERNEX_CODEX_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("codex"));
        Self {
            executable,
            arguments: vec!["app-server".into(), "--listen".into(), "stdio://".into()],
        }
    }
}

#[derive(Debug, Error)]
pub enum CodexAppServerError {
    #[error("could not start Codex App Server with {program}: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("Codex App Server did not expose its {0} pipe")]
    MissingPipe(&'static str),
    #[error("Codex App Server I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Codex App Server returned invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("Codex App Server emitted a JSON line larger than {MAX_JSON_LINE_BYTES} bytes")]
    MessageTooLarge,
    #[error("Codex App Server closed the connection unexpectedly")]
    Closed,
    #[error("Codex App Server request `{method}` failed ({code}): {message}")]
    Rpc {
        method: String,
        code: i64,
        message: String,
    },
    #[error("Codex App Server response for `{0}` did not match the documented schema")]
    InvalidResponse(String),
    #[error("Codex App Server timed out while waiting for {0}")]
    Timeout(&'static str),
    #[error("could not open the ChatGPT sign-in page: {0}")]
    Browser(String),
    #[error("ChatGPT sign-in failed: {0}")]
    LoginFailed(String),
    #[error("Codex turn failed: {0}")]
    TurnFailed(String),
    #[error("Codex turn was interrupted")]
    Interrupted,
    #[error("Codex turn was cancelled by the user")]
    Cancelled,
    #[error("Codex task cannot be empty")]
    EmptyTask,
}

/// Sanitized account information returned by App Server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccount {
    #[serde(rename = "type")]
    pub account_type: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub plan_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccountStatus {
    pub account: Option<CodexAccount>,
    pub requires_openai_auth: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRateLimitWindow {
    pub used_percent: u64,
    #[serde(default)]
    pub window_duration_mins: Option<u64>,
    #[serde(default)]
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexCredits {
    #[serde(default)]
    pub has_credits: bool,
    #[serde(default)]
    pub unlimited: bool,
    #[serde(default)]
    pub balance: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRateLimitSnapshot {
    #[serde(default)]
    pub limit_id: Option<String>,
    #[serde(default)]
    pub limit_name: Option<String>,
    #[serde(default)]
    pub plan_type: Option<String>,
    #[serde(default)]
    pub primary: Option<CodexRateLimitWindow>,
    #[serde(default)]
    pub secondary: Option<CodexRateLimitWindow>,
    #[serde(default)]
    pub credits: Option<CodexCredits>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRateLimitResetCredits {
    #[serde(default)]
    pub available_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRateLimits {
    #[serde(default)]
    pub rate_limits: Option<CodexRateLimitSnapshot>,
    #[serde(default)]
    pub rate_limits_by_limit_id: Option<BTreeMap<String, CodexRateLimitSnapshot>>,
    #[serde(default)]
    pub rate_limit_reset_credits: Option<CodexRateLimitResetCredits>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLoginStart {
    pub login_id: String,
    pub auth_url: String,
}

/// Low-level initialized App Server connection. Requests are sequential, while notifications and
/// server requests received ahead of a response are retained for the caller.
pub struct CodexAppServerClient {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    pending: VecDeque<Value>,
    next_id: u64,
}

impl CodexAppServerClient {
    pub async fn connect() -> Result<Self, CodexAppServerError> {
        Self::connect_with(CodexAppServerConfig::default()).await
    }

    pub async fn connect_with(config: CodexAppServerConfig) -> Result<Self, CodexAppServerError> {
        let program = config.executable.display().to_string();
        let mut command = Command::new(&config.executable);
        command
            .args(&config.arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|source| CodexAppServerError::Spawn { program, source })?;
        let Some(stdin) = child.stdin.take() else {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(CodexAppServerError::MissingPipe("stdin"));
        };
        let Some(stdout) = child.stdout.take() else {
            drop(stdin);
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(CodexAppServerError::MissingPipe("stdout"));
        };
        let mut client = Self {
            child: Some(child),
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
            pending: VecDeque::new(),
            next_id: 1,
        };
        let initialized = client
            .request(
                "initialize",
                Some(json!({
                    "clientInfo": {
                        "name": "kernex",
                        "title": "Kernex",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": {}
                })),
            )
            .await;
        if let Err(error) = initialized {
            let _ = client.shutdown().await;
            return Err(error);
        }
        if let Err(error) = client.notify("initialized", None).await {
            let _ = client.shutdown().await;
            return Err(error);
        }
        Ok(client)
    }

    pub async fn account(
        &mut self,
        refresh_token: bool,
    ) -> Result<CodexAccountStatus, CodexAppServerError> {
        let value = self
            .request("account/read", Some(json!({"refreshToken": refresh_token})))
            .await?;
        serde_json::from_value(value)
            .map_err(|_| CodexAppServerError::InvalidResponse("account/read".into()))
    }

    pub async fn rate_limits(&mut self) -> Result<CodexRateLimits, CodexAppServerError> {
        let value = self.request("account/rateLimits/read", None).await?;
        serde_json::from_value(value)
            .map_err(|_| CodexAppServerError::InvalidResponse("account/rateLimits/read".into()))
    }

    pub async fn models(&mut self) -> Result<Vec<ProviderModel>, CodexAppServerError> {
        let mut models = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let value = self
                .request(
                    "model/list",
                    Some(json!({
                        "limit": 100,
                        "cursor": cursor,
                        "includeHidden": false
                    })),
                )
                .await?;
            let data = value
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| CodexAppServerError::InvalidResponse("model/list".into()))?;
            models.extend(data.iter().filter_map(|model| {
                Some(ProviderModel {
                    id: model.get("id")?.as_str()?.to_owned(),
                    display_name: model
                        .get("displayName")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    description: model
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    is_default: model
                        .get("isDefault")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    owned_by: Some("openai".into()),
                    input_token_limit: None,
                    output_token_limit: None,
                })
            }));
            cursor = value
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        models.sort_by(|left, right| {
            right
                .is_default
                .cmp(&left.is_default)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(models)
    }

    pub async fn start_chatgpt_login(&mut self) -> Result<CodexLoginStart, CodexAppServerError> {
        let value = self
            .request(
                "account/login/start",
                Some(json!({
                    "type": "chatgpt",
                    "useHostedLoginSuccessPage": true,
                    "appBrand": "chatgpt"
                })),
            )
            .await?;
        let login_id = value
            .get("loginId")
            .and_then(Value::as_str)
            .ok_or_else(|| CodexAppServerError::InvalidResponse("account/login/start".into()))?;
        let auth_url = value
            .get("authUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| CodexAppServerError::InvalidResponse("account/login/start".into()))?;
        Ok(CodexLoginStart {
            login_id: login_id.to_owned(),
            auth_url: auth_url.to_owned(),
        })
    }

    pub async fn wait_for_login(
        &mut self,
        login_id: &str,
    ) -> Result<CodexAccountStatus, CodexAppServerError> {
        let deadline = tokio::time::Instant::now() + LOGIN_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(CodexAppServerError::Timeout("ChatGPT sign-in"));
            }
            let message = tokio::time::timeout(remaining, self.next_message())
                .await
                .map_err(|_| CodexAppServerError::Timeout("ChatGPT sign-in"))??;
            if message.get("method").and_then(Value::as_str) == Some("account/login/completed")
                && message.pointer("/params/loginId").and_then(Value::as_str) == Some(login_id)
            {
                if message
                    .pointer("/params/success")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    return self.account(false).await;
                }
                let error = message
                    .pointer("/params/error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown authentication error");
                return Err(CodexAppServerError::LoginFailed(error.to_owned()));
            }
            if message.get("id").is_some() && message.get("method").is_some() {
                self.respond_error(
                    message.get("id").cloned().unwrap_or(Value::Null),
                    -32601,
                    "Kernex does not support this server request during login",
                )
                .await?;
            }
        }
    }

    pub async fn logout(&mut self) -> Result<(), CodexAppServerError> {
        self.request("account/logout", None).await.map(|_| ())
    }

    pub async fn shutdown(&mut self) -> Result<(), CodexAppServerError> {
        self.stdin.take();
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        match tokio::time::timeout(Duration::from_millis(500), child.wait()).await {
            Ok(result) => {
                result?;
            }
            Err(_) => {
                child.kill().await?;
                child.wait().await?;
            }
        }
        Ok(())
    }

    async fn request(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, CodexAppServerError> {
        let id = Value::from(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.write_request(id.clone(), method, params).await?;
        let response = tokio::time::timeout(REQUEST_TIMEOUT, async {
            loop {
                let message = self.read_message().await?;
                if message.get("id") == Some(&id) {
                    if let Some(error) = message.get("error") {
                        return Err(CodexAppServerError::Rpc {
                            method: method.to_owned(),
                            code: error.get("code").and_then(Value::as_i64).unwrap_or(-1),
                            message: error
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown JSON-RPC error")
                                .to_owned(),
                        });
                    }
                    return message
                        .get("result")
                        .cloned()
                        .ok_or_else(|| CodexAppServerError::InvalidResponse(method.to_owned()));
                }
                self.pending.push_back(message);
            }
        })
        .await
        .map_err(|_| CodexAppServerError::Timeout("a JSON-RPC response"))??;
        Ok(response)
    }

    async fn write_request(
        &mut self,
        id: Value,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), CodexAppServerError> {
        let mut message = Map::from_iter([
            ("id".into(), id),
            ("method".into(), Value::String(method.to_owned())),
        ]);
        if let Some(params) = params {
            message.insert("params".into(), params);
        }
        self.write_message(&Value::Object(message)).await
    }

    async fn notify(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), CodexAppServerError> {
        let mut message = Map::from_iter([("method".into(), Value::String(method.to_owned()))]);
        if let Some(params) = params {
            message.insert("params".into(), params);
        }
        self.write_message(&Value::Object(message)).await
    }

    async fn respond(&mut self, id: Value, result: Value) -> Result<(), CodexAppServerError> {
        self.write_message(&json!({"id": id, "result": result}))
            .await
    }

    async fn respond_error(
        &mut self,
        id: Value,
        code: i64,
        message: &str,
    ) -> Result<(), CodexAppServerError> {
        self.write_message(&json!({
            "id": id,
            "error": {"code": code, "message": message}
        }))
        .await
    }

    async fn write_message(&mut self, message: &Value) -> Result<(), CodexAppServerError> {
        let bytes = serde_json::to_vec(message)?;
        if bytes.len() > MAX_JSON_LINE_BYTES {
            return Err(CodexAppServerError::MessageTooLarge);
        }
        let stdin = self.stdin.as_mut().ok_or(CodexAppServerError::Closed)?;
        stdin.write_all(&bytes).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn next_message(&mut self) -> Result<Value, CodexAppServerError> {
        if let Some(message) = self.pending.pop_front() {
            return Ok(message);
        }
        self.read_message().await
    }

    async fn read_message(&mut self) -> Result<Value, CodexAppServerError> {
        loop {
            let mut bytes = Vec::new();
            let read = (&mut self.stdout)
                .take((MAX_JSON_LINE_BYTES + 1) as u64)
                .read_until(b'\n', &mut bytes)
                .await?;
            if read == 0 {
                return Err(CodexAppServerError::Closed);
            }
            if bytes.len() > MAX_JSON_LINE_BYTES {
                return Err(CodexAppServerError::MessageTooLarge);
            }
            while matches!(bytes.last(), Some(b'\n' | b'\r')) {
                bytes.pop();
            }
            if bytes.is_empty() {
                continue;
            }
            return Ok(serde_json::from_slice(&bytes)?);
        }
    }
}

impl Drop for CodexAppServerClient {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let _ = child.start_kill();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = child.wait().await;
            });
        }
    }
}

pub async fn codex_account_status(
    refresh_token: bool,
) -> Result<CodexAccountStatus, CodexAppServerError> {
    let mut client = CodexAppServerClient::connect().await?;
    let result = client.account(refresh_token).await;
    let _ = client.shutdown().await;
    result
}

pub async fn codex_rate_limits() -> Result<CodexRateLimits, CodexAppServerError> {
    let mut client = CodexAppServerClient::connect().await?;
    let result = client.rate_limits().await;
    let _ = client.shutdown().await;
    result
}

pub async fn codex_models() -> Result<Vec<ProviderModel>, CodexAppServerError> {
    let mut client = CodexAppServerClient::connect().await?;
    let result = client.models().await;
    let _ = client.shutdown().await;
    result
}

pub async fn codex_login_chatgpt() -> Result<CodexAccountStatus, CodexAppServerError> {
    let mut client = CodexAppServerClient::connect().await?;
    let result = async {
        let login = client.start_chatgpt_login().await?;
        webbrowser::open(&login.auth_url)
            .map_err(|error| CodexAppServerError::Browser(error.to_string()))?;
        client.wait_for_login(&login.login_id).await
    }
    .await;
    let _ = client.shutdown().await;
    result
}

pub async fn codex_logout() -> Result<(), CodexAppServerError> {
    let mut client = CodexAppServerClient::connect().await?;
    let result = client.logout().await;
    let _ = client.shutdown().await;
    result
}

/// Inputs needed to start or resume one App Server turn.
#[derive(Debug, Clone)]
pub struct CodexTurnConfig {
    pub workspace: PathBuf,
    pub model: String,
    pub permission_mode: PermissionMode,
    pub provider_thread_id: Option<String>,
}

pub async fn run_codex_turn(
    config: CodexTurnConfig,
    task: impl Into<String>,
    history: Vec<Message>,
    approver: Arc<dyn Approver>,
    events: Arc<dyn EventSink>,
    cancelled: Arc<AtomicBool>,
) -> Result<AgentRunResult, CodexAppServerError> {
    run_codex_turn_with_server(
        CodexAppServerConfig::default(),
        config,
        task,
        history,
        approver,
        events,
        cancelled,
    )
    .await
}

/// Variant used by protocol tests and embedders that supply a known App Server executable.
pub async fn run_codex_turn_with_server(
    server: CodexAppServerConfig,
    config: CodexTurnConfig,
    task: impl Into<String>,
    history: Vec<Message>,
    approver: Arc<dyn Approver>,
    events: Arc<dyn EventSink>,
    cancelled: Arc<AtomicBool>,
) -> Result<AgentRunResult, CodexAppServerError> {
    let task = task.into();
    if task.trim().is_empty() {
        return Err(CodexAppServerError::EmptyTask);
    }
    if cancelled.load(Ordering::Relaxed) {
        return Err(CodexAppServerError::Cancelled);
    }
    events.emit(AgentEvent::Started {
        task: task.clone(),
        provider: "codex".into(),
        model: config.model.clone(),
    });
    let mut client = tokio::select! {
        client = CodexAppServerClient::connect_with(server) => client?,
        _ = wait_for_cancellation(&cancelled) => return Err(CodexAppServerError::Cancelled),
    };
    let result = run_connected_codex_turn(
        &mut client,
        config,
        task,
        history,
        approver,
        events,
        cancelled,
    )
    .await;
    let _ = client.shutdown().await;
    result
}

async fn run_connected_codex_turn(
    client: &mut CodexAppServerClient,
    config: CodexTurnConfig,
    task: String,
    mut history: Vec<Message>,
    approver: Arc<dyn Approver>,
    events: Arc<dyn EventSink>,
    cancelled: Arc<AtomicBool>,
) -> Result<AgentRunResult, CodexAppServerError> {
    let (approval_policy, sandbox) = codex_permissions(config.permission_mode);
    let model = (!config.model.trim().is_empty()).then_some(config.model.clone());
    let thread_request = async {
        if let Some(thread_id) = &config.provider_thread_id {
            client
                .request(
                    "thread/resume",
                    Some(json!({
                        "threadId": thread_id,
                        "cwd": config.workspace,
                        "model": model,
                        "approvalPolicy": approval_policy,
                        "sandbox": sandbox
                    })),
                )
                .await
        } else {
            client
                .request(
                    "thread/start",
                    Some(json!({
                        "cwd": config.workspace,
                        "model": model,
                        "approvalPolicy": approval_policy,
                        "sandbox": sandbox,
                        "serviceName": "kernex"
                    })),
                )
                .await
        }
    };
    let thread_result = tokio::select! {
        result = thread_request => result,
        _ = wait_for_cancellation(&cancelled) => return Err(CodexAppServerError::Cancelled),
    };
    let thread_result = thread_result?;
    let thread_id = thread_result
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .ok_or_else(|| CodexAppServerError::InvalidResponse("thread start or resume".into()))?
        .to_owned();

    events.emit(AgentEvent::ModelRequested { step: 1 });
    let turn_result = tokio::select! {
        result = client.request(
                "turn/start",
                Some(json!({
                    "threadId": thread_id,
                    "input": [{"type": "text", "text": task}]
                })),
            ) => result?,
        _ = wait_for_cancellation(&cancelled) => return Err(CodexAppServerError::Cancelled),
    };
    let turn_id = turn_result
        .pointer("/turn/id")
        .and_then(Value::as_str)
        .ok_or_else(|| CodexAppServerError::InvalidResponse("turn/start".into()))?
        .to_owned();
    let mut final_answer = None;
    let mut last_agent_message = None;
    let mut token_usage = TokenUsage::default();
    let mut latest_error = None;

    let outcome = loop {
        let message = tokio::select! {
            message = client.next_message() => message?,
            _ = wait_for_cancellation(&cancelled) => {
                let id = Value::from(client.next_id);
                client.next_id = client.next_id.saturating_add(1);
                let _ = client
                    .write_request(id, "turn/interrupt", Some(json!({"threadId": thread_id, "turnId": turn_id})))
                    .await;
                break Err(CodexAppServerError::Cancelled);
            }
        };
        if message.get("id").is_some() && message.get("method").is_some() {
            handle_server_request(client, &message, approver.as_ref()).await?;
            continue;
        }
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            continue;
        };
        match method {
            "item/agentMessage/delta" => {
                if let Some(delta) = message.pointer("/params/delta").and_then(Value::as_str) {
                    events.emit(AgentEvent::ModelDelta {
                        step: 1,
                        event: ProviderStreamEvent::TextDelta {
                            text: delta.to_owned(),
                        },
                    });
                }
            }
            "thread/tokenUsage/updated" => {
                token_usage.input_tokens = message
                    .pointer("/params/tokenUsage/last/inputTokens")
                    .and_then(Value::as_u64);
                token_usage.output_tokens = message
                    .pointer("/params/tokenUsage/last/outputTokens")
                    .and_then(Value::as_u64);
            }
            "item/started" => emit_item_started(&events, &message),
            "item/completed" => {
                if let Some(item) = message.pointer("/params/item") {
                    match item.get("type").and_then(Value::as_str) {
                        Some("agentMessage") => {
                            if let Some(text) = item.get("text").and_then(Value::as_str) {
                                last_agent_message = Some(text.to_owned());
                                if item.get("phase").and_then(Value::as_str) == Some("final_answer")
                                {
                                    final_answer = Some(text.to_owned());
                                }
                            }
                        }
                        Some("commandExecution" | "fileChange") => {
                            emit_item_completed(&events, item)
                        }
                        _ => {}
                    }
                }
            }
            "error" => {
                latest_error = message
                    .pointer("/params/error/message")
                    .or_else(|| message.pointer("/params/message"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            "turn/completed" => {
                let status = message
                    .pointer("/params/turn/status")
                    .and_then(Value::as_str)
                    .unwrap_or("failed");
                match status {
                    "completed" => break Ok(()),
                    "interrupted" => break Err(CodexAppServerError::Interrupted),
                    _ => {
                        let error = message
                            .pointer("/params/turn/error/message")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .or(latest_error)
                            .unwrap_or_else(|| format!("turn completed with status {status}"));
                        break Err(CodexAppServerError::TurnFailed(error));
                    }
                }
            }
            _ => {}
        }
    };

    outcome?;
    let final_answer = final_answer.or(last_agent_message).unwrap_or_default();
    events.emit(AgentEvent::ModelResponded {
        step: 1,
        content: final_answer.clone(),
        tool_calls: Vec::new(),
    });
    events.emit(AgentEvent::Completed { steps: 1 });
    if !history
        .last()
        .is_some_and(|message| message.role == Role::User && message.content == task)
    {
        history.push(Message::new(Role::User, task));
    }
    history.push(Message::new(Role::Assistant, final_answer.clone()));
    Ok(AgentRunResult {
        final_answer,
        steps: 1,
        messages: history,
        token_usage,
        provider_thread_id: Some(thread_id),
    })
}

fn codex_permissions(mode: PermissionMode) -> (&'static str, &'static str) {
    match mode {
        PermissionMode::ReadOnly => ("never", "read-only"),
        PermissionMode::Ask => ("untrusted", "read-only"),
        PermissionMode::AutoSafe => ("on-request", "workspace-write"),
        PermissionMode::FullAccess => ("never", "danger-full-access"),
    }
}

async fn handle_server_request(
    client: &mut CodexAppServerClient,
    message: &Value,
    approver: &dyn Approver,
) -> Result<(), CodexAppServerError> {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").unwrap_or(&Value::Null);
    match method {
        "item/commandExecution/requestApproval" => {
            let network = params
                .get("networkApprovalContext")
                .is_some_and(|value| !value.is_null());
            let resource = if network {
                params
                    .pointer("/networkApprovalContext/host")
                    .and_then(Value::as_str)
                    .unwrap_or("network destination")
            } else {
                params
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex command")
            };
            let request = PermissionRequest {
                capability: if network {
                    Capability::NetworkRequest
                } else {
                    Capability::ExecuteCommand
                },
                risk: RiskLevel::High,
                summary: params
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex requests permission to execute a command")
                    .to_owned(),
                resource: resource.to_owned(),
                details: approval_details(params),
            };
            let decision = codex_approval_decision(approver.decide(&request));
            client.respond(id, json!({"decision": decision})).await
        }
        "item/fileChange/requestApproval" => {
            let request = PermissionRequest {
                capability: Capability::WriteFile,
                risk: RiskLevel::High,
                summary: params
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex requests permission to change project files")
                    .to_owned(),
                resource: params
                    .get("grantRoot")
                    .and_then(Value::as_str)
                    .unwrap_or("project files")
                    .to_owned(),
                details: approval_details(params),
            };
            let decision = codex_approval_decision(approver.decide(&request));
            client.respond(id, json!({"decision": decision})).await
        }
        "item/permissions/requestApproval" => {
            let request = PermissionRequest {
                capability: if params
                    .pointer("/permissions/network")
                    .is_some_and(|v| !v.is_null())
                {
                    Capability::NetworkRequest
                } else {
                    Capability::WriteFile
                },
                risk: RiskLevel::High,
                summary: params
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex requests additional sandbox permissions")
                    .to_owned(),
                resource: params
                    .get("cwd")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex sandbox")
                    .to_owned(),
                details: vec![
                    params
                        .get("permissions")
                        .cloned()
                        .unwrap_or_default()
                        .to_string(),
                ],
            };
            let decision = approver.decide(&request);
            let (permissions, scope) = match decision {
                PermissionDecision::AllowOnce => (
                    params
                        .get("permissions")
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                    "turn",
                ),
                PermissionDecision::AllowForSession | PermissionDecision::AllowForProject => (
                    params
                        .get("permissions")
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                    "session",
                ),
                PermissionDecision::Deny => (json!({}), "turn"),
            };
            client
                .respond(id, json!({"permissions": permissions, "scope": scope}))
                .await
        }
        "tool/requestUserInput" => client.respond(id, json!({"answers": {}})).await,
        "mcpServer/elicitation/request" => {
            client
                .respond(id, json!({"action": "decline", "content": null}))
                .await
        }
        _ => {
            client
                .respond_error(id, -32601, "Kernex does not support this server request")
                .await
        }
    }
}

fn approval_details(params: &Value) -> Vec<String> {
    ["cwd", "turnId", "itemId"]
        .into_iter()
        .filter_map(|key| {
            params
                .get(key)
                .and_then(Value::as_str)
                .map(|value| format!("{key}: {value}"))
        })
        .collect()
}

fn codex_approval_decision(decision: PermissionDecision) -> &'static str {
    match decision {
        PermissionDecision::AllowOnce => "accept",
        PermissionDecision::AllowForSession | PermissionDecision::AllowForProject => {
            "acceptForSession"
        }
        PermissionDecision::Deny => "decline",
    }
}

fn emit_item_started(events: &Arc<dyn EventSink>, message: &Value) {
    let Some(item) = message.pointer("/params/item") else {
        return;
    };
    let Some(call) = observable_item(item) else {
        return;
    };
    events.emit(AgentEvent::ToolStarted { step: 1, call });
}

fn emit_item_completed(events: &Arc<dyn EventSink>, item: &Value) {
    let Some(call) = observable_item(item) else {
        return;
    };
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("failed");
    if status == "completed" {
        let (result, diff) = if item.get("type").and_then(Value::as_str) == Some("commandExecution")
        {
            (
                item.get("aggregatedOutput")
                    .and_then(Value::as_str)
                    .unwrap_or("Command completed")
                    .to_owned(),
                None,
            )
        } else {
            let changes = item.get("changes").cloned().unwrap_or_else(|| json!([]));
            let diff = changes.as_array().map(|changes| {
                changes
                    .iter()
                    .filter_map(|change| change.get("diff").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            (changes.to_string(), diff.filter(|value| !value.is_empty()))
        };
        events.emit(AgentEvent::ToolFinished {
            step: 1,
            call_id: call.id,
            name: call.name,
            result,
            diff,
        });
    } else {
        events.emit(AgentEvent::ToolFailed {
            step: 1,
            call_id: call.id,
            name: call.name,
            error: format!("Codex item completed with status {status}"),
        });
    }
}

fn observable_item(item: &Value) -> Option<ToolCall> {
    let id = item.get("id")?.as_str()?.to_owned();
    match item.get("type")?.as_str()? {
        "commandExecution" => Some(ToolCall {
            id,
            name: "codex_command".into(),
            arguments: json!({
                "command": item.get("command"),
                "cwd": item.get("cwd")
            }),
        }),
        "fileChange" => Some(ToolCall {
            id,
            name: "codex_file_change".into(),
            arguments: json!({"changes": item.get("changes")}),
        }),
        _ => None,
    }
}

async fn wait_for_cancellation(cancelled: &AtomicBool) {
    while !cancelled.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
