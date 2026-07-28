use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use url::{Host, Url};

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
    OpenAiCompatible,
    Anthropic,
    Gemini,
    Local,
    Custom,
}

impl ProviderKind {
    pub const ALL: [Self; 5] = [
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
    /// Maps an HTTP header name to an environment variable containing its value.
    #[serde(default)]
    pub header_env: BTreeMap<String, String>,
}

impl ProviderConfig {
    pub fn for_kind(kind: ProviderKind, model: impl Into<String>) -> Self {
        let (base_url, api_key_env) = match kind {
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
            header_env: BTreeMap::new(),
        }
    }

    pub fn endpoint(&self) -> Result<String, ProviderError> {
        if self.base_url.trim().is_empty() {
            return Err(ProviderError::MissingBaseUrl(self.kind));
        }
        let base = self.base_url.trim_end_matches('/');
        match self.kind {
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
}

pub type ProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<CompletionResponse, ProviderError>> + Send + 'a>>;

/// Object-safe model interface consumed by both user-facing applications.
pub trait ModelProvider: Send + Sync {
    fn config(&self) -> &ProviderConfig;
    fn complete(&self, request: CompletionRequest) -> ProviderFuture<'_>;
}

pub struct HttpModelProvider {
    config: ProviderConfig,
    client: reqwest::Client,
    permissions: Arc<PermissionGate>,
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
        })
    }

    async fn send(&self, request: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let endpoint = self.config.endpoint()?;
        self.permissions.authorize(&PermissionRequest {
            capability: Capability::NetworkRequest,
            risk: RiskLevel::Medium,
            summary: format!("contact {} model {}", self.config.kind, self.config.model),
            resource: endpoint.clone(),
            details: vec!["Conversation content will be sent to the configured provider".into()],
        })?;
        if let Some(variable) = &self.config.api_key_env {
            self.permissions.authorize(&PermissionRequest {
                capability: Capability::AccessSecret,
                risk: RiskLevel::Medium,
                summary: format!("read API credential from {variable}"),
                resource: variable.clone(),
                details: vec!["The credential value will not be logged or persisted".into()],
            })?;
        }

        let api_key = self.config.api_key()?;
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

        let payload = request_payload(&self.config, &request);
        let mut builder = self.client.post(endpoint).headers(headers).json(&payload);
        match self.config.kind {
            ProviderKind::Anthropic => {
                builder = builder.header("anthropic-version", "2023-06-01");
                if let Some(key) = api_key {
                    builder = builder.header("x-api-key", key);
                }
            }
            ProviderKind::Gemini => {
                if let Some(key) = api_key {
                    builder = builder.header("x-goog-api-key", key);
                }
            }
            ProviderKind::OpenAiCompatible | ProviderKind::Local | ProviderKind::Custom => {
                if let Some(key) = api_key {
                    builder = builder.bearer_auth(key);
                }
            }
        }

        let response = builder.send().await?;
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
}

fn request_payload(config: &ProviderConfig, request: &CompletionRequest) -> Value {
    match config.kind {
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
    use super::*;

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
    }

    #[test]
    fn defaults_use_expected_api_key_variables() {
        let config = ProviderConfig::for_kind(ProviderKind::Gemini, "gemini-test");
        assert_eq!(config.api_key_env.as_deref(), Some("GEMINI_API_KEY"));
        assert!(config.endpoint().unwrap().contains("gemini-test"));
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
    fn recognizes_loopback_provider_urls() {
        assert!(is_loopback_url("http://127.0.0.1:11434/v1"));
        assert!(is_loopback_url("http://[::1]:8080/v1"));
        assert!(is_loopback_url("http://localhost:1234/v1"));
        assert!(!is_loopback_url("https://api.example.com/v1"));
    }
}
