use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use url::{Host, Url};

use crate::auth::SecretValue;
use crate::permission::{
    Capability, PermissionError, PermissionGate, PermissionRequest, RiskLevel,
};

const MAX_ERROR_BODY_BYTES: usize = 16 * 1024;
const MAX_RESPONSE_BODY_BYTES: usize = 32 * 1024 * 1024;
const PROVIDER_CONNECT_TIMEOUT_SECONDS: u64 = 30;
const PROVIDER_REQUEST_TIMEOUT_SECONDS: u64 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    Codex,
    OpenAiCompatible,
    Anthropic,
    Gemini,
    Local,
    Custom,
}

impl ProviderKind {
    pub const ALL: [Self; 6] = [
        Self::Codex,
        Self::OpenAiCompatible,
        Self::Anthropic,
        Self::Gemini,
        Self::Local,
        Self::Custom,
    ];
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Codex => "codex",
            Self::OpenAiCompatible => "openai-compatible",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::Local => "local",
            Self::Custom => "custom",
        })
    }
}

impl FromStr for ProviderKind {
    type Err = ProviderError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "codex" | "chatgpt" | "openai-subscription" => Ok(Self::Codex),
            "openai" | "openai-compatible" | "open_ai_compatible" => Ok(Self::OpenAiCompatible),
            "anthropic" | "claude" => Ok(Self::Anthropic),
            "gemini" | "google" => Ok(Self::Gemini),
            "local" | "ollama" | "lm-studio" => Ok(Self::Local),
            "custom" => Ok(Self::Custom),
            _ => Err(ProviderError::UnknownKind(value.to_owned())),
        }
    }
}

/// Provider settings store only the environment-variable name, never an API key value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub model: String,
    pub base_url: String,
    pub api_key_env: Option<String>,
    /// Name of a secure authentication profile resolved by the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_profile: Option<String>,
    /// Maps an HTTP header name to an environment variable containing its value.
    #[serde(default)]
    pub header_env: BTreeMap<String, String>,
}

impl ProviderConfig {
    pub fn for_kind(kind: ProviderKind, model: impl Into<String>) -> Self {
        let (base_url, api_key_env) = match kind {
            ProviderKind::Codex => (String::new(), None),
            ProviderKind::OpenAiCompatible => (
                "https://api.openai.com/v1".to_owned(),
                Some("OPENAI_API_KEY".to_owned()),
            ),
            ProviderKind::Anthropic => (
                "https://api.anthropic.com/v1".to_owned(),
                Some("ANTHROPIC_API_KEY".to_owned()),
            ),
            ProviderKind::Gemini => (
                "https://generativelanguage.googleapis.com/v1beta".to_owned(),
                Some("GEMINI_API_KEY".to_owned()),
            ),
            ProviderKind::Local => ("http://127.0.0.1:11434/v1".to_owned(), None),
            ProviderKind::Custom => (String::new(), None),
        };
        Self {
            kind,
            model: model.into(),
            base_url,
            api_key_env,
            auth_profile: None,
            header_env: BTreeMap::new(),
        }
    }

    pub fn endpoint(&self) -> Result<String, ProviderError> {
        if self.kind == ProviderKind::Codex {
            return Err(ProviderError::NotHttpProvider(self.kind));
        }
        if self.base_url.trim().is_empty() {
            return Err(ProviderError::MissingBaseUrl(self.kind));
        }
        let base = self.base_url.trim_end_matches('/');
        match self.kind {
            ProviderKind::Codex => Err(ProviderError::NotHttpProvider(self.kind)),
            ProviderKind::OpenAiCompatible | ProviderKind::Local | ProviderKind::Custom => {
                Ok(format!("{base}/chat/completions"))
            }
            ProviderKind::Anthropic => Ok(format!("{base}/messages")),
            ProviderKind::Gemini => Ok(format!("{base}/models/{}:generateContent", self.model)),
        }
    }

    fn api_key(&self) -> Result<Option<String>, ProviderError> {
        let Some(variable) = &self.api_key_env else {
            return Ok(None);
        };
        env::var(variable)
            .map(Some)
            .map_err(|_| ProviderError::MissingApiKey(variable.clone()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub risk_level: RiskLevel,
    pub permission: Capability,
    pub supports_cancellation: bool,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

impl Message {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            name: None,
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }

    pub fn tool_result(call: &ToolCall, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            name: Some(call.name.clone()),
            tool_call_id: Some(call.id.clone()),
            tool_calls: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub messages: Vec<Message>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
}

impl CompletionRequest {
    pub fn new(messages: Vec<Message>) -> Self {
        Self {
            messages,
            max_tokens: Some(4096),
            temperature: Some(0.2),
            tools: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

impl TokenUsage {
    pub fn add(&mut self, other: &Self) {
        self.input_tokens = add_optional(self.input_tokens, other.input_tokens);
        self.output_tokens = add_optional(self.output_tokens, other.output_tokens);
    }
}

fn add_optional(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub content: String,
    pub model: Option<String>,
    pub usage: TokenUsage,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("unknown provider kind: {0}")]
    UnknownKind(String),
    #[error("provider {0} requires a base URL")]
    MissingBaseUrl(ProviderKind),
    #[error("provider {0} is not an HTTP completion provider")]
    NotHttpProvider(ProviderKind),
    #[error("model name cannot be empty")]
    MissingModel,
    #[error("API key environment variable is not set: {0}")]
    MissingApiKey(String),
    #[error("invalid custom header {name}: {reason}")]
    InvalidHeader { name: String, reason: String },
    #[error(transparent)]
    Permission(#[from] PermissionError),
    #[error("provider request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("provider returned invalid JSON: {0}")]
    InvalidResponse(#[from] serde_json::Error),
    #[error("provider returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("provider response did not contain generated text")]
    MissingContent,
    #[error("provider response exceeded the {0}-byte limit")]
    ResponseTooLarge(usize),
    #[error("provider stream was invalid: {0}")]
    InvalidStream(String),
}

pub type ProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<CompletionResponse, ProviderError>> + Send + 'a>>;

pub type ProviderModelsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<ProviderModel>, ProviderError>> + Send + 'a>>;

/// Provider-neutral metadata returned by a model catalog endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderModel {
    pub id: String,
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub is_default: bool,
    pub owned_by: Option<String>,
    pub input_token_limit: Option<u64>,
    pub output_token_limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderStreamEvent {
    TextDelta {
        text: String,
    },
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: String,
    },
    Usage {
        usage: TokenUsage,
    },
}

pub trait ProviderStreamSink: Send + Sync {
    fn emit(&self, event: ProviderStreamEvent);
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderCapabilities {
    pub text_streaming: bool,
    pub tool_call_streaming: bool,
    pub token_usage: bool,
    pub cancellation: bool,
    pub model_discovery: bool,
    pub oauth_pkce: bool,
    pub reasoning_options: bool,
}

/// Object-safe model interface consumed by both user-facing applications.
pub trait ModelProvider: Send + Sync {
    fn config(&self) -> &ProviderConfig;
    fn complete(&self, request: CompletionRequest) -> ProviderFuture<'_>;

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            text_streaming: false,
            tool_call_streaming: false,
            token_usage: true,
            cancellation: true,
            model_discovery: false,
            oauth_pkce: false,
            reasoning_options: false,
        }
    }

    fn models(&self) -> ProviderModelsFuture<'_> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn stream(
        &self,
        request: CompletionRequest,
        sink: Arc<dyn ProviderStreamSink>,
    ) -> ProviderFuture<'_> {
        Box::pin(async move {
            let response = self.complete(request).await?;
            if !response.content.is_empty() {
                sink.emit(ProviderStreamEvent::TextDelta {
                    text: response.content.clone(),
                });
            }
            sink.emit(ProviderStreamEvent::Usage {
                usage: response.usage.clone(),
            });
            Ok(response)
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderCredentialKind {
    ApiKey,
    OAuthBearer,
}

struct ProviderCredential {
    value: SecretValue,
    kind: ProviderCredentialKind,
}

pub struct HttpModelProvider {
    config: ProviderConfig,
    client: reqwest::Client,
    permissions: Arc<PermissionGate>,
    credential: Option<ProviderCredential>,
    google_resource_project: Option<String>,
}

impl HttpModelProvider {
    pub fn new(
        config: ProviderConfig,
        permissions: Arc<PermissionGate>,
    ) -> Result<Self, ProviderError> {
        if config.model.trim().is_empty() {
            return Err(ProviderError::MissingModel);
        }
        config.endpoint()?;
        let mut client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(PROVIDER_CONNECT_TIMEOUT_SECONDS))
            .timeout(Duration::from_secs(PROVIDER_REQUEST_TIMEOUT_SECONDS));
        if is_loopback_url(&config.base_url) {
            client = client.no_proxy();
        }
        Ok(Self {
            config,
            client: client.build()?,
            permissions,
            credential: None,
            google_resource_project: None,
        })
    }

    pub fn with_credential(
        mut self,
        credential: SecretValue,
        kind: ProviderCredentialKind,
    ) -> Self {
        self.credential = Some(ProviderCredential {
            value: credential,
            kind,
        });
        self
    }

    pub fn with_google_resource_project(mut self, project: impl Into<String>) -> Self {
        self.google_resource_project = Some(project.into());
        self
    }

    async fn send(&self, request: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let endpoint = self.config.endpoint()?;
        let payload = request_payload(&self.config, &request);
        let response = self
            .authenticated_request(&endpoint, &payload)?
            .send()
            .await?;
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BODY_BYTES as u64)
        {
            return Err(ProviderError::ResponseTooLarge(MAX_RESPONSE_BODY_BYTES));
        }
        let body = response.bytes().await?;
        if body.len() > MAX_RESPONSE_BODY_BYTES {
            return Err(ProviderError::ResponseTooLarge(MAX_RESPONSE_BODY_BYTES));
        }
        if !status.is_success() {
            let selected = &body[..body.len().min(MAX_ERROR_BODY_BYTES)];
            return Err(ProviderError::Http {
                status: status.as_u16(),
                body: String::from_utf8_lossy(selected).into_owned(),
            });
        }
        let value: Value = serde_json::from_slice(&body)?;
        parse_response(self.config.kind, value)
    }

    async fn discover_models(&self) -> Result<Vec<ProviderModel>, ProviderError> {
        let endpoint = format!("{}/models", self.config.base_url.trim_end_matches('/'));
        let response = self.authenticated_get(&endpoint)?.send().await?;
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BODY_BYTES as u64)
        {
            return Err(ProviderError::ResponseTooLarge(MAX_RESPONSE_BODY_BYTES));
        }
        let body = response.bytes().await?;
        if body.len() > MAX_RESPONSE_BODY_BYTES {
            return Err(ProviderError::ResponseTooLarge(MAX_RESPONSE_BODY_BYTES));
        }
        if !status.is_success() {
            let selected = &body[..body.len().min(MAX_ERROR_BODY_BYTES)];
            return Err(ProviderError::Http {
                status: status.as_u16(),
                body: String::from_utf8_lossy(selected).into_owned(),
            });
        }
        parse_models(self.config.kind, serde_json::from_slice(&body)?)
    }

    fn authenticated_request(
        &self,
        endpoint: &str,
        payload: &Value,
    ) -> Result<reqwest::RequestBuilder, ProviderError> {
        self.authenticated_builder(endpoint, self.client.post(endpoint).json(payload))
    }

    fn authenticated_get(&self, endpoint: &str) -> Result<reqwest::RequestBuilder, ProviderError> {
        self.authenticated_builder(endpoint, self.client.get(endpoint))
    }

    fn authenticated_builder(
        &self,
        endpoint: &str,
        builder: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, ProviderError> {
        self.permissions.authorize(&PermissionRequest {
            capability: Capability::NetworkRequest,
            risk: RiskLevel::Medium,
            summary: format!("contact {} model {}", self.config.kind, self.config.model),
            resource: endpoint.to_owned(),
            details: vec!["Conversation content will be sent to the configured provider".into()],
        })?;
        if let Some(profile) = &self.config.auth_profile {
            self.permissions.authorize(&PermissionRequest {
                capability: Capability::AccessSecret,
                risk: RiskLevel::Medium,
                summary: format!("use secure authentication profile {profile}"),
                resource: profile.clone(),
                details: vec!["The credential value will not be logged or persisted".into()],
            })?;
        } else if let Some(variable) = &self.config.api_key_env {
            self.permissions.authorize(&PermissionRequest {
                capability: Capability::AccessSecret,
                risk: RiskLevel::Medium,
                summary: format!("read API credential from {variable}"),
                resource: variable.clone(),
                details: vec!["The credential value will not be logged or persisted".into()],
            })?;
        }

        let environment_credential = if self.credential.is_none() {
            self.config.api_key()?.map(SecretValue::new)
        } else {
            None
        };
        let credential = self
            .credential
            .as_ref()
            .map(|credential| (&credential.value, credential.kind))
            .or_else(|| {
                environment_credential
                    .as_ref()
                    .map(|credential| (credential, ProviderCredentialKind::ApiKey))
            });
        let mut headers = HeaderMap::new();
        for (name, variable) in &self.config.header_env {
            self.permissions.authorize(&PermissionRequest {
                capability: Capability::AccessSecret,
                risk: RiskLevel::Medium,
                summary: format!("read custom provider credential from {variable}"),
                resource: variable.clone(),
                details: vec![format!("Value will be sent only in the {name} header")],
            })?;
            let value =
                env::var(variable).map_err(|_| ProviderError::MissingApiKey(variable.clone()))?;
            let parsed_name = HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
                ProviderError::InvalidHeader {
                    name: name.clone(),
                    reason: error.to_string(),
                }
            })?;
            let parsed_value =
                HeaderValue::from_str(&value).map_err(|error| ProviderError::InvalidHeader {
                    name: name.clone(),
                    reason: error.to_string(),
                })?;
            headers.insert(parsed_name, parsed_value);
        }

        let mut builder = builder.headers(headers);
        match self.config.kind {
            ProviderKind::Codex => {
                return Err(ProviderError::NotHttpProvider(self.config.kind));
            }
            ProviderKind::Anthropic => {
                builder = builder.header("anthropic-version", "2023-06-01");
                if let Some((credential, kind)) = credential {
                    builder = match kind {
                        ProviderCredentialKind::ApiKey => {
                            builder.header("x-api-key", credential.expose())
                        }
                        ProviderCredentialKind::OAuthBearer => {
                            builder.bearer_auth(credential.expose())
                        }
                    };
                }
            }
            ProviderKind::Gemini => {
                if let Some(project) = &self.google_resource_project {
                    let value = HeaderValue::from_str(project).map_err(|error| {
                        ProviderError::InvalidHeader {
                            name: "x-goog-user-project".into(),
                            reason: error.to_string(),
                        }
                    })?;
                    builder = builder.header("x-goog-user-project", value);
                }
                if let Some((credential, kind)) = credential {
                    builder = match kind {
                        ProviderCredentialKind::ApiKey => {
                            builder.header("x-goog-api-key", credential.expose())
                        }
                        ProviderCredentialKind::OAuthBearer => {
                            builder.bearer_auth(credential.expose())
                        }
                    };
                }
            }
            ProviderKind::OpenAiCompatible | ProviderKind::Local | ProviderKind::Custom => {
                if let Some((credential, _)) = credential {
                    builder = builder.bearer_auth(credential.expose());
                }
            }
        }

        Ok(builder)
    }

    async fn send_stream(
        &self,
        request: CompletionRequest,
        sink: Arc<dyn ProviderStreamSink>,
    ) -> Result<CompletionResponse, ProviderError> {
        let endpoint = match self.config.kind {
            ProviderKind::Gemini => format!(
                "{}/models/{}:streamGenerateContent?alt=sse",
                self.config.base_url.trim_end_matches('/'),
                self.config.model
            ),
            _ => self.config.endpoint()?,
        };
        let mut payload = request_payload(&self.config, &request);
        if self.config.kind != ProviderKind::Gemini {
            payload["stream"] = Value::Bool(true);
            if matches!(
                self.config.kind,
                ProviderKind::OpenAiCompatible | ProviderKind::Local | ProviderKind::Custom
            ) {
                payload["stream_options"] = json!({"include_usage": true});
            }
        }
        let response = self
            .authenticated_request(&endpoint, &payload)?
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.bytes().await?;
            let selected = &body[..body.len().min(MAX_ERROR_BODY_BYTES)];
            return Err(ProviderError::Http {
                status: status.as_u16(),
                body: String::from_utf8_lossy(selected).into_owned(),
            });
        }
        let is_event_stream = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("text/event-stream"));
        if !is_event_stream {
            let body = response.bytes().await?;
            if body.len() > MAX_RESPONSE_BODY_BYTES {
                return Err(ProviderError::ResponseTooLarge(MAX_RESPONSE_BODY_BYTES));
            }
            let parsed = parse_response(self.config.kind, serde_json::from_slice(&body)?)?;
            emit_complete_response(&parsed, &sink);
            return Ok(parsed);
        }

        let mut decoder = SseDecoder::default();
        let mut accumulator = StreamAccumulator::default();
        let mut body = response.bytes_stream();
        while let Some(chunk) = body.next().await {
            for data in decoder.push(&chunk?)? {
                if data == "[DONE]" {
                    continue;
                }
                let value: Value = serde_json::from_str(&data)?;
                accumulator.consume(self.config.kind, &value, &sink)?;
            }
        }
        for data in decoder.finish()? {
            if data != "[DONE]" {
                let value: Value = serde_json::from_str(&data)?;
                accumulator.consume(self.config.kind, &value, &sink)?;
            }
        }
        accumulator.finish()
    }
}

#[derive(Default)]
struct SseDecoder {
    bytes: Vec<u8>,
    total: usize,
}

impl SseDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, ProviderError> {
        self.total = self.total.saturating_add(chunk.len());
        if self.total > MAX_RESPONSE_BODY_BYTES {
            return Err(ProviderError::ResponseTooLarge(MAX_RESPONSE_BODY_BYTES));
        }
        self.bytes.extend_from_slice(chunk);
        self.frames(false)
    }

    fn finish(&mut self) -> Result<Vec<String>, ProviderError> {
        self.frames(true)
    }

    fn frames(&mut self, include_remainder: bool) -> Result<Vec<String>, ProviderError> {
        let mut output = Vec::new();
        while let Some((position, separator_length)) = find_sse_separator(&self.bytes) {
            let frame = self.bytes.drain(..position).collect::<Vec<_>>();
            self.bytes.drain(..separator_length);
            if let Some(data) = sse_data(&frame)? {
                output.push(data);
            }
        }
        if include_remainder && !self.bytes.is_empty() {
            let frame = std::mem::take(&mut self.bytes);
            if let Some(data) = sse_data(&frame)? {
                output.push(data);
            }
        }
        Ok(output)
    }
}

fn find_sse_separator(bytes: &[u8]) -> Option<(usize, usize)> {
    let crlf = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| (position, 4));
    let lf = bytes
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|position| (position, 2));
    match (crlf, lf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(separator), None) | (None, Some(separator)) => Some(separator),
        (None, None) => None,
    }
}

fn sse_data(frame: &[u8]) -> Result<Option<String>, ProviderError> {
    let frame = std::str::from_utf8(frame)
        .map_err(|error| ProviderError::InvalidStream(error.to_string()))?;
    let data = frame
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join("\n");
    Ok((!data.is_empty()).then_some(data))
}

#[derive(Default)]
struct PartialToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Default)]
struct StreamAccumulator {
    content: String,
    model: Option<String>,
    usage: TokenUsage,
    tool_calls: BTreeMap<usize, PartialToolCall>,
}

impl StreamAccumulator {
    fn consume(
        &mut self,
        kind: ProviderKind,
        value: &Value,
        sink: &Arc<dyn ProviderStreamSink>,
    ) -> Result<(), ProviderError> {
        match kind {
            ProviderKind::Codex => Err(ProviderError::NotHttpProvider(kind)),
            ProviderKind::OpenAiCompatible | ProviderKind::Local | ProviderKind::Custom => {
                self.consume_openai(value, sink)
            }
            ProviderKind::Anthropic => self.consume_anthropic(value, sink),
            ProviderKind::Gemini => self.consume_gemini(value, sink),
        }
    }

    fn consume_openai(
        &mut self,
        value: &Value,
        sink: &Arc<dyn ProviderStreamSink>,
    ) -> Result<(), ProviderError> {
        if self.model.is_none() {
            self.model = value
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        if let Some(usage) = value.get("usage") {
            self.usage.input_tokens = usage.get("prompt_tokens").and_then(Value::as_u64);
            self.usage.output_tokens = usage.get("completion_tokens").and_then(Value::as_u64);
            sink.emit(ProviderStreamEvent::Usage {
                usage: self.usage.clone(),
            });
        }
        for choice in value
            .get("choices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let delta = choice.get("delta").unwrap_or(choice);
            if let Some(text) = delta.get("content").and_then(Value::as_str) {
                self.push_text(text, sink);
            }
            for tool in delta
                .get("tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let index = tool.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let function = tool.get("function").unwrap_or(&Value::Null);
                self.push_tool_delta(
                    index,
                    tool.get("id").and_then(Value::as_str),
                    function.get("name").and_then(Value::as_str),
                    function
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    sink,
                );
            }
        }
        Ok(())
    }

    fn consume_anthropic(
        &mut self,
        value: &Value,
        sink: &Arc<dyn ProviderStreamSink>,
    ) -> Result<(), ProviderError> {
        match value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "message_start" => {
                let message = value.get("message").unwrap_or(&Value::Null);
                self.model = message
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                self.usage.input_tokens = message
                    .get("usage")
                    .and_then(|usage| usage.get("input_tokens"))
                    .and_then(Value::as_u64);
            }
            "content_block_start" => {
                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let block = value.get("content_block").unwrap_or(&Value::Null);
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(text) = block.get("text").and_then(Value::as_str) {
                            self.push_text(text, sink);
                        }
                    }
                    Some("tool_use") => self.push_tool_delta(
                        index,
                        block.get("id").and_then(Value::as_str),
                        block.get("name").and_then(Value::as_str),
                        block
                            .get("input")
                            .filter(|input| !input.is_null())
                            .map(Value::to_string)
                            .as_deref()
                            .unwrap_or_default(),
                        sink,
                    ),
                    _ => {}
                }
            }
            "content_block_delta" => {
                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let delta = value.get("delta").unwrap_or(&Value::Null);
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str) {
                            self.push_text(text, sink);
                        }
                    }
                    Some("input_json_delta") => self.push_tool_delta(
                        index,
                        None,
                        None,
                        delta
                            .get("partial_json")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        sink,
                    ),
                    _ => {}
                }
            }
            "message_delta" => {
                self.usage.output_tokens = value
                    .get("usage")
                    .and_then(|usage| usage.get("output_tokens"))
                    .and_then(Value::as_u64);
                sink.emit(ProviderStreamEvent::Usage {
                    usage: self.usage.clone(),
                });
            }
            "error" => {
                return Err(ProviderError::InvalidStream(
                    value
                        .get("error")
                        .and_then(|error| error.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or("provider stream returned an error")
                        .to_owned(),
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn consume_gemini(
        &mut self,
        value: &Value,
        sink: &Arc<dyn ProviderStreamSink>,
    ) -> Result<(), ProviderError> {
        if self.model.is_none() {
            self.model = value
                .get("modelVersion")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        if let Some(usage) = value.get("usageMetadata") {
            self.usage.input_tokens = usage.get("promptTokenCount").and_then(Value::as_u64);
            self.usage.output_tokens = usage.get("candidatesTokenCount").and_then(Value::as_u64);
            sink.emit(ProviderStreamEvent::Usage {
                usage: self.usage.clone(),
            });
        }
        for part in value
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                self.push_text(text, sink);
            }
            if let Some(call) = part.get("functionCall") {
                let index = self.tool_calls.len();
                self.push_tool_delta(
                    index,
                    None,
                    call.get("name").and_then(Value::as_str),
                    &call
                        .get("args")
                        .cloned()
                        .unwrap_or_else(|| json!({}))
                        .to_string(),
                    sink,
                );
            }
        }
        Ok(())
    }

    fn push_text(&mut self, text: &str, sink: &Arc<dyn ProviderStreamSink>) {
        self.content.push_str(text);
        sink.emit(ProviderStreamEvent::TextDelta { text: text.into() });
    }

    fn push_tool_delta(
        &mut self,
        index: usize,
        id: Option<&str>,
        name: Option<&str>,
        arguments: &str,
        sink: &Arc<dyn ProviderStreamSink>,
    ) {
        let partial = self.tool_calls.entry(index).or_default();
        if let Some(id) = id {
            partial.id = Some(id.into());
        }
        if let Some(name) = name {
            partial.name = Some(name.into());
        }
        partial.arguments.push_str(arguments);
        sink.emit(ProviderStreamEvent::ToolCallDelta {
            index,
            id: id.map(str::to_owned),
            name: name.map(str::to_owned),
            arguments_delta: arguments.into(),
        });
    }

    fn finish(self) -> Result<CompletionResponse, ProviderError> {
        let tool_calls = self
            .tool_calls
            .into_iter()
            .map(|(index, call)| {
                let arguments = if call.arguments.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&call.arguments)?
                };
                Ok(ToolCall {
                    id: call.id.unwrap_or_else(|| format!("tool-{index}")),
                    name: call.name.ok_or(ProviderError::InvalidStream(format!(
                        "streamed tool call {index} had no name"
                    )))?,
                    arguments,
                })
            })
            .collect::<Result<Vec<_>, ProviderError>>()?;
        if self.content.is_empty() && tool_calls.is_empty() {
            return Err(ProviderError::MissingContent);
        }
        Ok(CompletionResponse {
            content: self.content,
            model: self.model,
            usage: self.usage,
            tool_calls,
        })
    }
}

fn emit_complete_response(response: &CompletionResponse, sink: &Arc<dyn ProviderStreamSink>) {
    if !response.content.is_empty() {
        sink.emit(ProviderStreamEvent::TextDelta {
            text: response.content.clone(),
        });
    }
    for (index, call) in response.tool_calls.iter().enumerate() {
        sink.emit(ProviderStreamEvent::ToolCallDelta {
            index,
            id: Some(call.id.clone()),
            name: Some(call.name.clone()),
            arguments_delta: call.arguments.to_string(),
        });
    }
    sink.emit(ProviderStreamEvent::Usage {
        usage: response.usage.clone(),
    });
}

fn is_loopback_url(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    match url.host() {
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

impl ModelProvider for HttpModelProvider {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    fn complete(&self, request: CompletionRequest) -> ProviderFuture<'_> {
        Box::pin(self.send(request))
    }

    fn models(&self) -> ProviderModelsFuture<'_> {
        Box::pin(self.discover_models())
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            text_streaming: true,
            tool_call_streaming: true,
            token_usage: true,
            cancellation: true,
            model_discovery: true,
            oauth_pkce: self.config.kind == ProviderKind::Gemini,
            reasoning_options: matches!(
                self.config.kind,
                ProviderKind::OpenAiCompatible | ProviderKind::Anthropic | ProviderKind::Gemini
            ),
        }
    }

    fn stream(
        &self,
        request: CompletionRequest,
        sink: Arc<dyn ProviderStreamSink>,
    ) -> ProviderFuture<'_> {
        Box::pin(self.send_stream(request, sink))
    }
}

fn parse_models(kind: ProviderKind, value: Value) -> Result<Vec<ProviderModel>, ProviderError> {
    let models = match kind {
        ProviderKind::Codex => None,
        ProviderKind::Gemini => value.get("models"),
        ProviderKind::OpenAiCompatible
        | ProviderKind::Anthropic
        | ProviderKind::Local
        | ProviderKind::Custom => value.get("data"),
    }
    .and_then(Value::as_array)
    .ok_or_else(|| ProviderError::InvalidStream("model catalog has no model list".into()))?;

    let mut parsed = models
        .iter()
        .filter_map(|model| {
            let raw_id = if kind == ProviderKind::Gemini {
                model.get("name")?.as_str()?
            } else {
                model.get("id")?.as_str()?
            };
            let id = raw_id.strip_prefix("models/").unwrap_or(raw_id).to_owned();
            Some(ProviderModel {
                id,
                display_name: model
                    .get("display_name")
                    .or_else(|| model.get("displayName"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                description: model
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                is_default: model
                    .get("is_default")
                    .or_else(|| model.get("isDefault"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                owned_by: model
                    .get("owned_by")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                input_token_limit: model
                    .get("input_token_limit")
                    .or_else(|| model.get("inputTokenLimit"))
                    .and_then(Value::as_u64),
                output_token_limit: model
                    .get("output_token_limit")
                    .or_else(|| model.get("outputTokenLimit"))
                    .and_then(Value::as_u64),
            })
        })
        .collect::<Vec<_>>();
    parsed.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(parsed)
}

fn request_payload(config: &ProviderConfig, request: &CompletionRequest) -> Value {
    match config.kind {
        ProviderKind::Codex => json!({}),
        ProviderKind::OpenAiCompatible | ProviderKind::Local | ProviderKind::Custom => {
            let messages: Vec<_> = request.messages.iter().map(openai_message).collect();
            let tools: Vec<_> = request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.input_schema,
                        }
                    })
                })
                .collect();
            omit_empty_tools(
                json!({
                    "model": config.model,
                    "messages": messages,
                    "max_tokens": request.max_tokens,
                    "temperature": request.temperature,
                    "tools": tools,
                }),
                request.tools.is_empty(),
            )
        }
        ProviderKind::Anthropic => {
            let system = request
                .messages
                .iter()
                .filter(|message| message.role == Role::System)
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            let messages: Vec<_> = request
                .messages
                .iter()
                .filter(|message| message.role != Role::System)
                .map(anthropic_message)
                .collect();
            let tools: Vec<_> = request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "input_schema": tool.input_schema,
                    })
                })
                .collect();
            omit_empty_tools(
                json!({
                    "model": config.model,
                    "system": system,
                    "messages": messages,
                    "max_tokens": request.max_tokens.unwrap_or(4096),
                    "temperature": request.temperature,
                    "tools": tools,
                }),
                request.tools.is_empty(),
            )
        }
        ProviderKind::Gemini => {
            let system = request
                .messages
                .iter()
                .filter(|message| message.role == Role::System)
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            let contents: Vec<_> = request
                .messages
                .iter()
                .filter(|message| message.role != Role::System)
                .map(gemini_message)
                .collect();
            let declarations: Vec<_> = request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema,
                    })
                })
                .collect();
            omit_empty_tools(
                json!({
                    "systemInstruction": { "parts": [{ "text": system }] },
                    "contents": contents,
                    "tools": [{ "functionDeclarations": declarations }],
                    "generationConfig": {
                        "maxOutputTokens": request.max_tokens,
                        "temperature": request.temperature,
                    },
                }),
                request.tools.is_empty(),
            )
        }
    }
}

fn omit_empty_tools(mut payload: Value, tools_are_empty: bool) -> Value {
    if tools_are_empty && let Some(object) = payload.as_object_mut() {
        object.remove("tools");
    }
    payload
}

fn openai_message(message: &Message) -> Value {
    match message.role {
        Role::Assistant => {
            let tool_calls: Vec<_> = message
                .tool_calls
                .iter()
                .map(|call| {
                    json!({
                        "id": call.id,
                        "type": "function",
                        "function": {
                            "name": call.name,
                            "arguments": call.arguments.to_string(),
                        }
                    })
                })
                .collect();
            json!({
                "role": "assistant",
                "content": if message.content.is_empty() { Value::Null } else { json!(message.content) },
                "tool_calls": tool_calls,
            })
        }
        Role::Tool => json!({
            "role": "tool",
            "content": message.content,
            "tool_call_id": message.tool_call_id,
            "name": message.name,
        }),
        Role::System | Role::User => json!({
            "role": if message.role == Role::System { "system" } else { "user" },
            "content": message.content,
            "name": message.name,
        }),
    }
}

fn anthropic_message(message: &Message) -> Value {
    match message.role {
        Role::Assistant => {
            let mut content = Vec::new();
            if !message.content.is_empty() {
                content.push(json!({"type": "text", "text": message.content}));
            }
            content.extend(message.tool_calls.iter().map(|call| {
                json!({
                    "type": "tool_use",
                    "id": call.id,
                    "name": call.name,
                    "input": call.arguments,
                })
            }));
            json!({"role": "assistant", "content": content})
        }
        Role::Tool => json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": message.tool_call_id,
                "content": message.content,
            }],
        }),
        Role::System | Role::User => json!({"role": "user", "content": message.content}),
    }
}

fn gemini_message(message: &Message) -> Value {
    match message.role {
        Role::Assistant => {
            let mut parts = Vec::new();
            if !message.content.is_empty() {
                parts.push(json!({"text": message.content}));
            }
            parts.extend(
                message.tool_calls.iter().map(
                    |call| json!({"functionCall": {"name": call.name, "args": call.arguments}}),
                ),
            );
            json!({"role": "model", "parts": parts})
        }
        Role::Tool => json!({
            "role": "user",
            "parts": [{"functionResponse": {
                "name": message.name,
                "response": {"result": message.content},
            }}],
        }),
        Role::System | Role::User => {
            json!({"role": "user", "parts": [{"text": message.content}]})
        }
    }
}

fn parse_response(kind: ProviderKind, value: Value) -> Result<CompletionResponse, ProviderError> {
    match kind {
        ProviderKind::Codex => Err(ProviderError::NotHttpProvider(kind)),
        ProviderKind::OpenAiCompatible | ProviderKind::Local | ProviderKind::Custom => {
            let content = value
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let tool_calls = value
                .pointer("/choices/0/message/tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|call| {
                    let arguments = call
                        .pointer("/function/arguments")
                        .and_then(Value::as_str)
                        .unwrap_or("{}");
                    Ok(ToolCall {
                        id: call
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        name: call
                            .pointer("/function/name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        arguments: serde_json::from_str(arguments)?,
                    })
                })
                .collect::<Result<Vec<_>, serde_json::Error>>()?;
            if content.is_empty() && tool_calls.is_empty() {
                return Err(ProviderError::MissingContent);
            }
            Ok(CompletionResponse {
                content,
                model: value
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                usage: TokenUsage {
                    input_tokens: value
                        .pointer("/usage/prompt_tokens")
                        .and_then(Value::as_u64),
                    output_tokens: value
                        .pointer("/usage/completion_tokens")
                        .and_then(Value::as_u64),
                },
                tool_calls,
            })
        }
        ProviderKind::Anthropic => {
            let blocks = value.get("content").and_then(Value::as_array);
            let content = blocks
                .into_iter()
                .flatten()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            let tool_calls = blocks
                .into_iter()
                .flatten()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
                .map(|block| ToolCall {
                    id: block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    name: block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    arguments: block.get("input").cloned().unwrap_or_else(|| json!({})),
                })
                .collect::<Vec<_>>();
            if content.is_empty() && tool_calls.is_empty() {
                return Err(ProviderError::MissingContent);
            }
            Ok(CompletionResponse {
                content,
                model: value
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                usage: TokenUsage {
                    input_tokens: value.pointer("/usage/input_tokens").and_then(Value::as_u64),
                    output_tokens: value
                        .pointer("/usage/output_tokens")
                        .and_then(Value::as_u64),
                },
                tool_calls,
            })
        }
        ProviderKind::Gemini => {
            let parts = value
                .pointer("/candidates/0/content/parts")
                .and_then(Value::as_array);
            let content = parts
                .into_iter()
                .flatten()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            let tool_calls = parts
                .into_iter()
                .flatten()
                .enumerate()
                .filter_map(|(index, part)| {
                    let call = part.get("functionCall")?;
                    let name = call.get("name")?.as_str()?.to_owned();
                    Some(ToolCall {
                        id: format!("gemini-{index}-{name}"),
                        name,
                        arguments: call.get("args").cloned().unwrap_or_else(|| json!({})),
                    })
                })
                .collect::<Vec<_>>();
            if content.is_empty() && tool_calls.is_empty() {
                return Err(ProviderError::MissingContent);
            }
            Ok(CompletionResponse {
                content,
                model: value
                    .get("modelVersion")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                usage: TokenUsage {
                    input_tokens: value
                        .pointer("/usageMetadata/promptTokenCount")
                        .and_then(Value::as_u64),
                    output_tokens: value
                        .pointer("/usageMetadata/candidatesTokenCount")
                        .and_then(Value::as_u64),
                },
                tool_calls,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct CapturingSink(Mutex<Vec<ProviderStreamEvent>>);

    impl ProviderStreamSink for CapturingSink {
        fn emit(&self, event: ProviderStreamEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[test]
    fn aliases_parse_to_provider_kinds() {
        assert_eq!(
            "openai".parse::<ProviderKind>().unwrap(),
            ProviderKind::OpenAiCompatible
        );
        assert_eq!(
            "claude".parse::<ProviderKind>().unwrap(),
            ProviderKind::Anthropic
        );
        assert_eq!(
            "ollama".parse::<ProviderKind>().unwrap(),
            ProviderKind::Local
        );
        assert_eq!(
            "chatgpt".parse::<ProviderKind>().unwrap(),
            ProviderKind::Codex
        );
    }

    #[test]
    fn defaults_use_expected_api_key_variables() {
        let config = ProviderConfig::for_kind(ProviderKind::Gemini, "gemini-test");
        assert_eq!(config.api_key_env.as_deref(), Some("GEMINI_API_KEY"));
        assert!(config.endpoint().unwrap().contains("gemini-test"));
        let codex = ProviderConfig::for_kind(ProviderKind::Codex, "gpt-test");
        assert!(codex.api_key_env.is_none());
        assert!(matches!(
            codex.endpoint(),
            Err(ProviderError::NotHttpProvider(ProviderKind::Codex))
        ));
    }

    #[test]
    fn parses_openai_compatible_response() {
        let response = parse_response(
            ProviderKind::OpenAiCompatible,
            json!({
                "model": "test-model",
                "choices": [{"message": {"content": "done"}}],
                "usage": {"prompt_tokens": 2, "completion_tokens": 1}
            }),
        )
        .unwrap();
        assert_eq!(response.content, "done");
        assert_eq!(response.usage.input_tokens, Some(2));
    }

    #[test]
    fn parses_anthropic_text_blocks() {
        let response = parse_response(
            ProviderKind::Anthropic,
            json!({"content": [{"type": "text", "text": "first"}, {"type": "text", "text": "second"}]}),
        )
        .unwrap();
        assert_eq!(response.content, "first\nsecond");
    }

    #[test]
    fn parses_gemini_parts() {
        let response = parse_response(
            ProviderKind::Gemini,
            json!({"candidates": [{"content": {"parts": [{"text": "answer"}]}}]}),
        )
        .unwrap();
        assert_eq!(response.content, "answer");
    }

    #[test]
    fn parses_tool_calls_from_all_provider_shapes() {
        let openai = parse_response(
            ProviderKind::OpenAiCompatible,
            json!({"choices": [{"message": {"content": null, "tool_calls": [{
                "id": "one",
                "function": {"name": "read_file", "arguments": "{\"path\":\"README.md\"}"}
            }]}}]}),
        )
        .unwrap();
        assert_eq!(openai.tool_calls[0].name, "read_file");

        let anthropic = parse_response(
            ProviderKind::Anthropic,
            json!({"content": [{
                "type": "tool_use",
                "id": "two",
                "name": "git_status",
                "input": {}
            }]}),
        )
        .unwrap();
        assert_eq!(anthropic.tool_calls[0].id, "two");

        let gemini = parse_response(
            ProviderKind::Gemini,
            json!({"candidates": [{"content": {"parts": [{
                "functionCall": {"name": "search_files", "args": {"query": "needle"}}
            }]}}]}),
        )
        .unwrap();
        assert_eq!(gemini.tool_calls[0].arguments["query"], "needle");
    }

    #[test]
    fn one_shot_payload_omits_empty_tools() {
        let config = ProviderConfig::for_kind(ProviderKind::Local, "test");
        let request = CompletionRequest::new(vec![Message::new(Role::User, "hello")]);
        let payload = request_payload(&config, &request);
        assert!(payload.get("tools").is_none());
    }

    #[test]
    fn sse_decoder_handles_split_frames_and_multiline_data() {
        let mut decoder = SseDecoder::default();
        assert!(
            decoder
                .push(b"event: message\ndata: {\"one\":")
                .unwrap()
                .is_empty()
        );
        let frames = decoder
            .push(b"1}\n\ndata: first\ndata: second\r\n\r\n")
            .unwrap();
        assert_eq!(frames, ["{\"one\":1}", "first\nsecond"]);
        assert!(decoder.finish().unwrap().is_empty());
    }

    #[test]
    fn streaming_accumulator_reassembles_openai_tool_calls() {
        let sink: Arc<dyn ProviderStreamSink> = Arc::new(CapturingSink::default());
        let mut accumulator = StreamAccumulator::default();
        accumulator
            .consume(
                ProviderKind::OpenAiCompatible,
                &json!({"model":"test","choices":[{"delta":{"content":"working ","tool_calls":[{"index":0,"id":"call-1","function":{"name":"read_file","arguments":"{\"path\":"}}]}}]}),
                &sink,
            )
            .unwrap();
        accumulator
            .consume(
                ProviderKind::OpenAiCompatible,
                &json!({"choices":[{"delta":{"content":"done","tool_calls":[{"index":0,"function":{"arguments":"\"README.md\"}"}}]}}],"usage":{"prompt_tokens":4,"completion_tokens":2}}),
                &sink,
            )
            .unwrap();
        let response = accumulator.finish().unwrap();
        assert_eq!(response.content, "working done");
        assert_eq!(response.tool_calls[0].name, "read_file");
        assert_eq!(response.tool_calls[0].arguments["path"], "README.md");
        assert_eq!(response.usage.output_tokens, Some(2));
    }

    #[test]
    fn parses_provider_model_catalogs() {
        let openai = parse_models(
            ProviderKind::OpenAiCompatible,
            json!({"data":[{"id":"z-model","owned_by":"vendor"},{"id":"a-model"}]}),
        )
        .unwrap();
        assert_eq!(openai[0].id, "a-model");
        assert_eq!(openai[1].owned_by.as_deref(), Some("vendor"));

        let gemini = parse_models(
            ProviderKind::Gemini,
            json!({"models":[{"name":"models/gemini-test","displayName":"Gemini Test","inputTokenLimit":123,"outputTokenLimit":45}]}),
        )
        .unwrap();
        assert_eq!(gemini[0].id, "gemini-test");
        assert_eq!(gemini[0].input_token_limit, Some(123));
    }

    #[test]
    fn recognizes_loopback_provider_urls() {
        assert!(is_loopback_url("http://127.0.0.1:11434/v1"));
        assert!(is_loopback_url("http://[::1]:8080/v1"));
        assert!(is_loopback_url("http://localhost:1234/v1"));
        assert!(!is_loopback_url("https://api.example.com/v1"));
    }
}
