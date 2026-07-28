//! Shared, provider-neutral runtime for the Kernex CLI and desktop applications.

pub mod agent;
pub mod command;
pub mod config;
pub mod diff;
pub mod git;
pub mod instructions;
pub mod lsp;
pub mod mcp;
pub mod permission;
pub mod plugin;
pub mod project;
pub mod provider;
pub mod syntax;
pub mod workspace;

/// The default behavioral contract sent to model providers.
pub const SYSTEM_PROMPT: &str = include_str!("../prompts/system.md");

/// Runtime version reported by release builds and protocol clients.
pub const VERSION: &str = match option_env!("KERNEX_RELEASE_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

pub use agent::{
    AgentEngine, AgentEvent, AgentRunResult, EventSink, NoopEventSink, ToolExecution, Toolbox,
};
pub use command::{CommandOutput, CommandRunner, CommandSpec};
pub use config::{KernexConfig, LanguageServerEntry};
pub use diff::{FileChange, FileEditor, unified_diff};
pub use git::GitRepository;
pub use instructions::{InstructionDocument, InstructionSet};
pub use lsp::{LanguageServerConfig, LspClient};
pub use mcp::{McpClient, McpServerConfig, McpTool};
pub use permission::{
    Approver, Capability, PermissionDecision, PermissionError, PermissionGate, PermissionPolicy,
    PermissionRequest, PermissionRule, RiskLevel,
};
pub use plugin::{PluginInfo, PluginRegistry, PluginTool};
pub use project::{FileRecord, ProjectIndex, SearchMatch};
pub use provider::{
    CompletionRequest, CompletionResponse, HttpModelProvider, Message, ModelProvider,
    ProviderConfig, ProviderKind, Role, TokenUsage, ToolCall, ToolDefinition,
};
pub use syntax::{SyntaxAnalyzer, SyntaxOutline, SyntaxSymbol};
pub use workspace::{Workspace, WorkspaceError};
