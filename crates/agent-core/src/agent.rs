use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::SYSTEM_PROMPT;
use crate::command::{CommandError, CommandRunner, CommandSpec};
use crate::config::LanguageServerEntry;
use crate::diff::{FileEditError, FileEditor};
use crate::git::{GitError, GitRepository};
use crate::instructions::{InstructionError, InstructionSet};
use crate::lsp::{LspClient, LspError};
use crate::mcp::{McpClient, McpError, McpServerConfig};
use crate::permission::{
    Capability, PermissionError, PermissionGate, PermissionRequest, RiskLevel,
};
use crate::plugin::{PluginError, PluginRegistry};
use crate::project::{ProjectError, ProjectIndex};
use crate::provider::{
    CompletionRequest, Message, ModelProvider, ProviderError, ProviderStreamEvent,
    ProviderStreamSink, Role, TokenUsage, ToolCall, ToolDefinition,
};
use crate::syntax::{SyntaxAnalyzer, SyntaxError};
use crate::workspace::{Workspace, WorkspaceError};

const DEFAULT_MAX_STEPS: usize = 24;
const DEFAULT_LIST_LIMIT: usize = 2_000;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    Started {
        task: String,
        provider: String,
        model: String,
    },
    ModelRequested {
        step: usize,
    },
    ModelDelta {
        step: usize,
        event: ProviderStreamEvent,
    },
    ModelResponded {
        step: usize,
        content: String,
        tool_calls: Vec<ToolCall>,
    },
    ToolStarted {
        step: usize,
        call: ToolCall,
    },
    ToolFinished {
        step: usize,
        call_id: String,
        name: String,
        result: String,
        diff: Option<String>,
    },
    ToolFailed {
        step: usize,
        call_id: String,
        name: String,
        error: String,
    },
    Completed {
        steps: usize,
    },
}

/// Receives every meaningful action for terminal logs, desktop timelines, or audit storage.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: AgentEvent);
}

pub struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn emit(&self, _event: AgentEvent) {}
}

#[derive(Debug, Clone)]
pub struct ToolExecution {
    pub content: String,
    pub diff: Option<String>,
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("unknown tool: {0}")]
    Unknown(String),
    #[error("invalid arguments for {tool}: {source}")]
    InvalidArguments {
        tool: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Permission(#[from] PermissionError),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Edit(#[from] FileEditError),
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error(transparent)]
    Syntax(#[from] SyntaxError),
    #[error(transparent)]
    Plugin(#[from] PluginError),
    #[error(transparent)]
    Mcp(#[from] McpError),
    #[error(transparent)]
    Lsp(#[from] LspError),
    #[error("no configured language server handles {0}")]
    NoLanguageServer(String),
    #[error("extension tools normalize to the same name: {0}")]
    DuplicateToolName(String),
    #[error("project index cache is unavailable")]
    IndexCache,
    #[error("no supported project command for {operation}")]
    NoProjectCommand { operation: String },
    #[error("could not serialize tool result: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Provider-neutral tool collection. Every path and process remains scoped to one workspace.
pub struct Toolbox {
    workspace: Arc<Workspace>,
    permissions: Arc<PermissionGate>,
    commands: CommandRunner,
    git: GitRepository,
    editor: FileEditor,
    plugins: PluginRegistry,
    mcp_tools: Vec<McpToolBinding>,
    lsp_servers: Vec<LspBinding>,
    project_index: Mutex<Option<ProjectIndex>>,
}

struct McpToolBinding {
    qualified_name: String,
    remote_name: String,
    definition: ToolDefinition,
    client: Arc<tokio::sync::Mutex<McpClient>>,
}

struct LspBinding {
    extensions: Vec<String>,
    client: Arc<tokio::sync::Mutex<LspClient>>,
}

impl Toolbox {
    pub fn new(
        workspace: Arc<Workspace>,
        permissions: Arc<PermissionGate>,
    ) -> Result<Self, ToolError> {
        let plugins = PluginRegistry::discover(&workspace)?;
        Ok(Self {
            commands: CommandRunner::new(workspace.clone(), permissions.clone()),
            git: GitRepository::new(workspace.clone(), permissions.clone()),
            editor: FileEditor::new(workspace.clone(), permissions.clone()),
            workspace,
            permissions,
            plugins,
            mcp_tools: Vec::new(),
            lsp_servers: Vec::new(),
            project_index: Mutex::new(None),
        })
    }

    pub async fn connect_mcp(
        mut self,
        configs: impl IntoIterator<Item = McpServerConfig>,
    ) -> Result<Self, ToolError> {
        for config in configs {
            let server_name = sanitize_tool_name(&config.name);
            let mut client =
                McpClient::connect(self.workspace.clone(), self.permissions.clone(), config)
                    .await?;
            let tools = client.list_tools().await?;
            let client = Arc::new(tokio::sync::Mutex::new(client));
            for remote in tools {
                let qualified_name =
                    format!("mcp__{}__{}", server_name, sanitize_tool_name(&remote.name));
                if self
                    .mcp_tools
                    .iter()
                    .any(|tool| tool.qualified_name == qualified_name)
                    || self.plugins.find(&qualified_name).is_some()
                {
                    return Err(ToolError::DuplicateToolName(qualified_name));
                }
                self.mcp_tools.push(McpToolBinding {
                    definition: ToolDefinition {
                        name: qualified_name.clone(),
                        description: remote.description.unwrap_or_else(|| {
                            format!("MCP tool {} from server {}", remote.name, server_name)
                        }),
                        input_schema: remote.input_schema,
                        output_schema: json!({"type": ["object", "array", "string"]}),
                        risk_level: RiskLevel::High,
                        permission: Capability::ExecuteCommand,
                        supports_cancellation: true,
                        timeout_seconds: Some(120),
                    },
                    qualified_name,
                    remote_name: remote.name,
                    client: client.clone(),
                });
            }
        }
        Ok(self)
    }

    pub async fn connect_language_servers(
        mut self,
        entries: impl IntoIterator<Item = LanguageServerEntry>,
    ) -> Result<Self, ToolError> {
        for entry in entries {
            let client = LspClient::connect(
                self.workspace.clone(),
                self.permissions.clone(),
                entry.server,
            )
            .await?;
            self.lsp_servers.push(LspBinding {
                extensions: entry
                    .extensions
                    .into_iter()
                    .map(|extension| extension.trim_start_matches('.').to_ascii_lowercase())
                    .collect(),
                client: Arc::new(tokio::sync::Mutex::new(client)),
            });
        }
        Ok(self)
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = vec![
            tool(
                "list_files",
                "List project files, respecting ignore rules.",
                json!({
                    "type": "object",
                    "properties": {"max_files": {"type": "integer", "minimum": 1, "maximum": 10000}},
                    "additionalProperties": false
                }),
            ),
            tool(
                "list_directory",
                "List files below a project directory, optionally recursively, while respecting ignore rules.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "recursive": {"type": "boolean"},
                        "max_entries": {"type": "integer", "minimum": 1, "maximum": 10000}
                    },
                    "additionalProperties": false
                }),
            ),
            tool(
                "read_file",
                "Read a UTF-8 text file inside the project.",
                json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"],
                    "additionalProperties": false
                }),
            ),
            tool(
                "search_files",
                "Search project text and return file, line, column, and preview.",
                json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"},
                        "case_sensitive": {"type": "boolean"},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 500}
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
            ),
            tool(
                "search_code",
                "Search source and project text and return bounded file, line, column, and preview records.",
                json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"},
                        "case_sensitive": {"type": "boolean"},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 500}
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
            ),
            tool(
                "write_file",
                "Create or replace a UTF-8 project file after showing its complete diff for approval.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "content": {"type": "string"}
                    },
                    "required": ["path", "content"],
                    "additionalProperties": false
                }),
            ),
            tool(
                "create_file",
                "Create a new UTF-8 project file after showing its complete diff; fail if it already exists.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "content": {"type": "string"}
                    },
                    "required": ["path", "content"],
                    "additionalProperties": false
                }),
            ),
            tool(
                "replace_in_file",
                "Replace an exact text span in a project file after showing the resulting diff for approval.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "old": {"type": "string"},
                        "new": {"type": "string"},
                        "replace_all": {"type": "boolean"}
                    },
                    "required": ["path", "old", "new"],
                    "additionalProperties": false
                }),
            ),
            tool(
                "apply_patch",
                "Apply one exact replacement to a project file after showing the resulting diff.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "old": {"type": "string"},
                        "new": {"type": "string"},
                        "replace_all": {"type": "boolean"}
                    },
                    "required": ["path", "old", "new"],
                    "additionalProperties": false
                }),
            ),
            tool(
                "execute_command",
                "Execute a program and explicit argument vector inside the project; no shell expansion is used.",
                json!({
                    "type": "object",
                    "properties": {
                        "program": {"type": "string"},
                        "args": {"type": "array", "items": {"type": "string"}},
                        "cwd": {"type": "string"},
                        "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": 1800}
                    },
                    "required": ["program"],
                    "additionalProperties": false
                }),
            ),
            tool(
                "run_command",
                "Run a program and explicit argument vector inside the project with bounded output and timeout.",
                json!({
                    "type": "object",
                    "properties": {
                        "program": {"type": "string"},
                        "args": {"type": "array", "items": {"type": "string"}},
                        "cwd": {"type": "string"},
                        "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": 1800}
                    },
                    "required": ["program"],
                    "additionalProperties": false
                }),
            ),
            tool(
                "git_status",
                "Show concise read-only Git status.",
                object_without_properties(),
            ),
            tool(
                "git_diff",
                "Show the current unstaged or staged Git diff for review.",
                json!({
                    "type": "object",
                    "properties": {"staged": {"type": "boolean"}},
                    "additionalProperties": false
                }),
            ),
            tool(
                "git_log",
                "Show a bounded, read-only Git commit history.",
                json!({
                    "type": "object",
                    "properties": {"max_entries": {"type": "integer", "minimum": 1, "maximum": 200}},
                    "additionalProperties": false
                }),
            ),
            project_command_tool("run_tests", "Run the project's detected test command."),
            project_command_tool("run_build", "Run the project's detected build command."),
            project_command_tool("run_formatter", "Run the project's detected formatter."),
            project_command_tool("run_linter", "Run the project's detected linter."),
            tool(
                "syntax_outline",
                "Parse a source file with tree-sitter and return named structural symbols and line ranges.",
                json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"],
                    "additionalProperties": false
                }),
            ),
        ];
        definitions.extend(self.plugins.definitions());
        definitions.extend(self.mcp_tools.iter().map(|tool| tool.definition.clone()));
        if !self.lsp_servers.is_empty() {
            definitions.push(tool(
                "lsp_symbols",
                "Ask the configured language server for semantic document symbols.",
                json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"],
                    "additionalProperties": false
                }),
            ));
        }
        definitions
    }

    pub async fn execute(&self, call: &ToolCall) -> Result<ToolExecution, ToolError> {
        match call.name.as_str() {
            "list_files" => {
                let args: ListFilesArgs = parse_args(call)?;
                self.authorize_read(Capability::SearchFiles, "list project files", ".")?;
                let max_files = args.max_files.unwrap_or(DEFAULT_LIST_LIMIT).min(10_000);
                let index = self.project_index()?;
                let total_files = index.files().len();
                let files = &index.files()[..total_files.min(max_files)];
                Ok(result(serde_json::to_string_pretty(&json!({
                    "files": files,
                    "total_files": total_files,
                    "truncated": total_files > files.len(),
                }))?))
            }
            "list_directory" => {
                let args: ListDirectoryArgs = parse_args(call)?;
                let directory = args.path.unwrap_or_else(|| ".".into());
                self.authorize_read(
                    Capability::SearchFiles,
                    "list project directory",
                    &directory,
                )?;
                let resolved = self.workspace.resolve_existing(&directory)?;
                if !resolved.is_dir() {
                    return Err(ToolError::Workspace(WorkspaceError::NotDirectory(resolved)));
                }
                let display_path = self.workspace.display_path(&resolved);
                let prefix = if display_path == "." {
                    String::new()
                } else {
                    display_path.trim_matches('/').to_owned()
                };
                let prefix_with_separator = (!prefix.is_empty()).then(|| format!("{prefix}/"));
                let max_entries = args.max_entries.unwrap_or(DEFAULT_LIST_LIMIT).min(10_000);
                let mut entries = std::collections::BTreeSet::new();
                let index = self.project_index()?;
                for file in index.files() {
                    let relative = match &prefix_with_separator {
                        Some(prefix) => match file.path.strip_prefix(prefix) {
                            Some(relative) => relative,
                            None => continue,
                        },
                        None => file.path.as_str(),
                    };
                    let entry = if args.recursive {
                        file.path.clone()
                    } else if let Some((directory, _)) = relative.split_once('/') {
                        match &prefix_with_separator {
                            Some(prefix) => format!("{prefix}{directory}/"),
                            None => format!("{directory}/"),
                        }
                    } else {
                        file.path.clone()
                    };
                    entries.insert(entry);
                    if entries.len() == max_entries {
                        break;
                    }
                }
                Ok(result(serde_json::to_string_pretty(&json!({
                    "path": display_path,
                    "entries": entries,
                    "truncated": entries.len() == max_entries,
                }))?))
            }
            "read_file" => {
                let args: ReadFileArgs = parse_args(call)?;
                let resolved = self.workspace.resolve_existing(&args.path)?;
                if self.workspace.is_sensitive_path(&resolved) {
                    self.permissions.authorize(&PermissionRequest {
                        capability: Capability::AccessSecret,
                        risk: RiskLevel::High,
                        summary: "read a potentially sensitive project file".into(),
                        resource: args.path.clone(),
                        details: vec![
                            "File contents may be returned to the configured model provider".into(),
                        ],
                    })?;
                } else {
                    self.authorize_read(Capability::ReadFile, "read project file", &args.path)?;
                }
                let content = self.workspace.read_text(&resolved)?;
                Ok(result(content))
            }
            "search_files" | "search_code" => {
                let args: SearchFilesArgs = parse_args(call)?;
                self.authorize_read(Capability::SearchFiles, "search project files", &args.query)?;
                let index = self.project_index()?;
                let matches = index.search(
                    &self.workspace,
                    &args.query,
                    args.case_sensitive,
                    args.max_results.unwrap_or(50).min(500),
                )?;
                Ok(result(serde_json::to_string_pretty(&matches)?))
            }
            "write_file" => {
                let args: WriteFileArgs = parse_args(call)?;
                let change = self.editor.write_text(&args.path, &args.content)?;
                self.invalidate_project_index();
                Ok(ToolExecution {
                    content: json!({
                        "path": change.path,
                        "bytes": change.after.len(),
                        "changed": change.before != change.after,
                    })
                    .to_string(),
                    diff: Some(change.diff),
                })
            }
            "create_file" => {
                let args: WriteFileArgs = parse_args(call)?;
                let change = self.editor.create_text(&args.path, &args.content)?;
                self.invalidate_project_index();
                Ok(ToolExecution {
                    content: json!({
                        "path": change.path,
                        "bytes": change.after.len(),
                        "created": true,
                    })
                    .to_string(),
                    diff: Some(change.diff),
                })
            }
            "replace_in_file" | "apply_patch" => {
                let args: ReplaceInFileArgs = parse_args(call)?;
                let change =
                    self.editor
                        .replace_text(&args.path, &args.old, &args.new, args.replace_all)?;
                self.invalidate_project_index();
                Ok(ToolExecution {
                    content: json!({
                        "path": change.path,
                        "bytes": change.after.len(),
                        "changed": change.before != change.after,
                    })
                    .to_string(),
                    diff: Some(change.diff),
                })
            }
            "execute_command" | "run_command" => {
                let args: ExecuteCommandArgs = parse_args(call)?;
                // Commands may mutate arbitrary project files, including before returning an error.
                self.invalidate_project_index();
                let mut command = CommandSpec::new(args.program, args.args);
                command.cwd = args.cwd.map(Into::into);
                command.timeout_seconds = args.timeout_seconds.unwrap_or(120).min(1800);
                let output = self.commands.run(&command).await?;
                Ok(result(serde_json::to_string_pretty(&output)?))
            }
            "git_status" => Ok(result(self.git.status()?.stdout)),
            "git_diff" => {
                let args: GitDiffArgs = parse_args(call)?;
                Ok(result(self.git.diff(args.staged)?.stdout))
            }
            "git_log" => {
                let args: GitLogArgs = parse_args(call)?;
                Ok(result(self.git.log(args.max_entries.unwrap_or(30))?.stdout))
            }
            "run_tests" | "run_build" | "run_formatter" | "run_linter" => {
                let args: ProjectCommandArgs = parse_args(call)?;
                self.invalidate_project_index();
                let mut command = self.project_command(&call.name, args.args)?;
                command.timeout_seconds = args.timeout_seconds.unwrap_or(600).min(1_800);
                let output = self.commands.run(&command).await?;
                Ok(result(serde_json::to_string_pretty(&output)?))
            }
            "syntax_outline" => {
                let args: ReadFileArgs = parse_args(call)?;
                self.authorize_read(Capability::ReadFile, "analyze source syntax", &args.path)?;
                let outline = SyntaxAnalyzer::outline(&self.workspace, args.path)?;
                Ok(result(serde_json::to_string_pretty(&outline)?))
            }
            "lsp_symbols" => {
                let args: ReadFileArgs = parse_args(call)?;
                self.authorize_read(Capability::ReadFile, "query semantic symbols", &args.path)?;
                let extension = std::path::Path::new(&args.path)
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let server = self
                    .lsp_servers
                    .iter()
                    .find(|server| server.extensions.iter().any(|item| item == &extension))
                    .ok_or_else(|| ToolError::NoLanguageServer(extension.clone()))?;
                let symbols = server
                    .client
                    .lock()
                    .await
                    .document_symbols(&args.path)
                    .await?;
                Ok(result(serde_json::to_string_pretty(&symbols)?))
            }
            name => {
                if let Some(plugin) = self.plugins.find(name) {
                    return Ok(result(
                        plugin.execute(&self.commands, &call.arguments).await?,
                    ));
                }
                if let Some(tool) = self
                    .mcp_tools
                    .iter()
                    .find(|tool| tool.qualified_name == name)
                {
                    let output = tool
                        .client
                        .lock()
                        .await
                        .call_tool(&tool.remote_name, call.arguments.clone())
                        .await?;
                    return Ok(result(serde_json::to_string_pretty(&output)?));
                }
                Err(ToolError::Unknown(name.to_owned()))
            }
        }
    }

    fn authorize_read(
        &self,
        capability: Capability,
        summary: &str,
        resource: &str,
    ) -> Result<(), PermissionError> {
        self.permissions.authorize(&PermissionRequest {
            capability,
            risk: RiskLevel::Low,
            summary: summary.to_owned(),
            resource: resource.to_owned(),
            details: Vec::new(),
        })
    }

    fn project_index(&self) -> Result<ProjectIndex, ToolError> {
        let mut cached = self
            .project_index
            .lock()
            .map_err(|_| ToolError::IndexCache)?;
        if cached.is_none() {
            *cached = Some(ProjectIndex::build(&self.workspace)?);
        }
        cached.clone().ok_or(ToolError::IndexCache)
    }

    fn invalidate_project_index(&self) {
        if let Ok(mut cached) = self.project_index.lock() {
            *cached = None;
        }
    }

    fn project_command(
        &self,
        operation: &str,
        additional_args: Vec<String>,
    ) -> Result<CommandSpec, ToolError> {
        let root = self.workspace.root();
        let (program, mut args): (&str, Vec<String>) = if root.join("Cargo.toml").is_file() {
            match operation {
                "run_tests" => ("cargo", vec!["test".into(), "--workspace".into()]),
                "run_build" => ("cargo", vec!["build".into(), "--workspace".into()]),
                "run_formatter" => ("cargo", vec!["fmt".into(), "--all".into()]),
                "run_linter" => (
                    "cargo",
                    vec![
                        "clippy".into(),
                        "--workspace".into(),
                        "--all-targets".into(),
                        "--all-features".into(),
                        "--".into(),
                        "-D".into(),
                        "warnings".into(),
                    ],
                ),
                _ => {
                    return Err(ToolError::NoProjectCommand {
                        operation: operation.into(),
                    });
                }
            }
        } else if root.join("package.json").is_file() {
            match operation {
                "run_tests" => ("npm", vec!["test".into(), "--".into(), "--run".into()]),
                "run_build" => ("npm", vec!["run".into(), "build".into()]),
                "run_formatter" => ("npm", vec!["run".into(), "format".into()]),
                "run_linter" => ("npm", vec!["run".into(), "lint".into()]),
                _ => {
                    return Err(ToolError::NoProjectCommand {
                        operation: operation.into(),
                    });
                }
            }
        } else {
            return Err(ToolError::NoProjectCommand {
                operation: operation.into(),
            });
        };
        args.extend(additional_args);
        Ok(CommandSpec::new(program, args))
    }
}

fn tool(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    let (permission, risk_level, timeout_seconds) = match name {
        "read_file" | "syntax_outline" | "lsp_symbols" => {
            (Capability::ReadFile, RiskLevel::Low, Some(30))
        }
        "list_files" | "list_directory" | "search_files" | "search_code" => {
            (Capability::SearchFiles, RiskLevel::Low, Some(30))
        }
        "git_status" | "git_diff" | "git_log" => (Capability::GitRead, RiskLevel::Low, Some(30)),
        "write_file" | "create_file" | "replace_in_file" | "apply_patch" => {
            (Capability::WriteFile, RiskLevel::Medium, Some(30))
        }
        _ => (Capability::ExecuteCommand, RiskLevel::Medium, Some(1_800)),
    };
    ToolDefinition {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema,
        output_schema: json!({"type": ["object", "array", "string"]}),
        risk_level,
        permission,
        supports_cancellation: true,
        timeout_seconds,
    }
}

fn project_command_tool(name: &str, description: &str) -> ToolDefinition {
    tool(
        name,
        description,
        json!({
            "type": "object",
            "properties": {
                "args": {"type": "array", "items": {"type": "string"}},
                "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": 1800}
            },
            "additionalProperties": false
        }),
    )
}

fn object_without_properties() -> Value {
    json!({"type": "object", "properties": {}, "additionalProperties": false})
}

fn sanitize_tool_name(name: &str) -> String {
    let mut sanitized = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        sanitized.push_str("unnamed");
    }
    sanitized
}

fn result(content: String) -> ToolExecution {
    ToolExecution {
        content,
        diff: None,
    }
}

fn parse_args<T: for<'de> Deserialize<'de>>(call: &ToolCall) -> Result<T, ToolError> {
    serde_json::from_value(call.arguments.clone()).map_err(|source| ToolError::InvalidArguments {
        tool: call.name.clone(),
        source,
    })
}

#[derive(Debug, Deserialize)]
struct ListFilesArgs {
    max_files: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ListDirectoryArgs {
    path: Option<String>,
    #[serde(default)]
    recursive: bool,
    max_entries: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ReadFileArgs {
    path: String,
}

#[derive(Debug, Deserialize)]
struct SearchFilesArgs {
    query: String,
    #[serde(default)]
    case_sensitive: bool,
    max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct WriteFileArgs {
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ReplaceInFileArgs {
    path: String,
    old: String,
    new: String,
    #[serde(default)]
    replace_all: bool,
}

#[derive(Debug, Deserialize)]
struct ExecuteCommandArgs {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    cwd: Option<String>,
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GitDiffArgs {
    #[serde(default)]
    staged: bool,
}

#[derive(Debug, Deserialize)]
struct GitLogArgs {
    max_entries: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ProjectCommandArgs {
    #[serde(default)]
    args: Vec<String>,
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct AgentRunResult {
    pub final_answer: String,
    pub steps: usize,
    pub messages: Vec<Message>,
    pub token_usage: TokenUsage,
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("agent task cannot be empty")]
    EmptyTask,
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Instructions(#[from] InstructionError),
    #[error("agent run was cancelled by the user")]
    Cancelled,
    #[error("agent reached its {0}-step safety limit before completing the task")]
    StepLimit(usize),
}

/// Iterative agent runtime shared by all presentation layers.
pub struct AgentEngine {
    provider: Arc<dyn ModelProvider>,
    toolbox: Toolbox,
    events: Arc<dyn EventSink>,
    max_steps: usize,
    cancelled: Arc<AtomicBool>,
}

impl AgentEngine {
    pub fn new(
        provider: Arc<dyn ModelProvider>,
        toolbox: Toolbox,
        events: Arc<dyn EventSink>,
    ) -> Self {
        Self {
            provider,
            toolbox,
            events,
            max_steps: DEFAULT_MAX_STEPS,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = max_steps.max(1);
        self
    }

    pub fn with_cancellation(mut self, cancelled: Arc<AtomicBool>) -> Self {
        self.cancelled = cancelled;
        self
    }

    pub async fn run(&self, task: impl Into<String>) -> Result<AgentRunResult, AgentError> {
        self.run_with_history(task, Vec::new()).await
    }

    pub async fn run_with_history(
        &self,
        task: impl Into<String>,
        history: Vec<Message>,
    ) -> Result<AgentRunResult, AgentError> {
        let task = task.into();
        if task.trim().is_empty() {
            return Err(AgentError::EmptyTask);
        }
        self.events.emit(AgentEvent::Started {
            task: task.clone(),
            provider: self.provider.config().kind.to_string(),
            model: self.provider.config().model.clone(),
        });

        let instructions = InstructionSet::discover(self.toolbox.workspace())?;
        let workspace_context = format!(
            "\n\nThe active project root is `{}`. All file paths and commands must remain inside this project.",
            self.toolbox.workspace().root().display()
        );
        let repository_instructions = instructions.render_for_prompt();
        let system_message = Message::new(
            Role::System,
            format!("{SYSTEM_PROMPT}{workspace_context}{repository_instructions}"),
        );
        let mut messages = history;
        if !messages
            .first()
            .is_some_and(|message| message.role == Role::System)
        {
            messages.insert(0, system_message);
        }
        messages.push(Message::new(Role::User, task));
        let tools = self.toolbox.definitions();
        let mut token_usage = TokenUsage::default();

        for step in 1..=self.max_steps {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(AgentError::Cancelled);
            }
            self.events.emit(AgentEvent::ModelRequested { step });
            let mut request = CompletionRequest::new(messages.clone());
            request.tools = tools.clone();
            let stream = Arc::new(AgentProviderStream {
                step,
                events: self.events.clone(),
            });
            let response = tokio::select! {
                response = self.provider.stream(request, stream) => response?,
                _ = wait_for_cancellation(&self.cancelled) => return Err(AgentError::Cancelled),
            };
            token_usage.add(&response.usage);
            self.events.emit(AgentEvent::ModelResponded {
                step,
                content: response.content.clone(),
                tool_calls: response
                    .tool_calls
                    .iter()
                    .map(observable_tool_call)
                    .collect(),
            });

            let mut assistant = Message::new(Role::Assistant, response.content.clone());
            assistant.tool_calls = response.tool_calls.clone();
            messages.push(assistant);

            if response.tool_calls.is_empty() {
                self.events.emit(AgentEvent::Completed { steps: step });
                return Ok(AgentRunResult {
                    final_answer: response.content,
                    steps: step,
                    messages,
                    token_usage,
                });
            }

            for call in response.tool_calls {
                if self.cancelled.load(Ordering::Relaxed) {
                    return Err(AgentError::Cancelled);
                }
                self.events.emit(AgentEvent::ToolStarted {
                    step,
                    call: observable_tool_call(&call),
                });
                let execution = tokio::select! {
                    execution = self.toolbox.execute(&call) => execution,
                    _ = wait_for_cancellation(&self.cancelled) => return Err(AgentError::Cancelled),
                };
                match execution {
                    Ok(execution) => {
                        self.events.emit(AgentEvent::ToolFinished {
                            step,
                            call_id: call.id.clone(),
                            name: call.name.clone(),
                            result: execution.content.clone(),
                            diff: execution.diff,
                        });
                        messages.push(Message::tool_result(&call, execution.content));
                    }
                    Err(error) => {
                        let error = error.to_string();
                        self.events.emit(AgentEvent::ToolFailed {
                            step,
                            call_id: call.id.clone(),
                            name: call.name.clone(),
                            error: error.clone(),
                        });
                        messages.push(Message::tool_result(
                            &call,
                            json!({"error": error}).to_string(),
                        ));
                    }
                }
            }
        }
        Err(AgentError::StepLimit(self.max_steps))
    }
}

struct AgentProviderStream {
    step: usize,
    events: Arc<dyn EventSink>,
}

impl ProviderStreamSink for AgentProviderStream {
    fn emit(&self, event: ProviderStreamEvent) {
        self.events.emit(AgentEvent::ModelDelta {
            step: self.step,
            event,
        });
    }
}

async fn wait_for_cancellation(cancelled: &AtomicBool) {
    while !cancelled.load(Ordering::Relaxed) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

fn observable_tool_call(call: &ToolCall) -> ToolCall {
    let arguments = match call.name.as_str() {
        "write_file" | "create_file" => json!({
            "path": call.arguments.get("path"),
            "content_bytes": call
                .arguments
                .get("content")
                .and_then(Value::as_str)
                .map(str::len),
        }),
        "replace_in_file" | "apply_patch" => json!({
            "path": call.arguments.get("path"),
            "old_bytes": call
                .arguments
                .get("old")
                .and_then(Value::as_str)
                .map(str::len),
            "new_bytes": call
                .arguments
                .get("new")
                .and_then(Value::as_str)
                .map(str::len),
            "replace_all": call.arguments.get("replace_all"),
        }),
        _ => call.arguments.clone(),
    };
    ToolCall {
        id: call.id.clone(),
        name: call.name.clone(),
        arguments,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use crate::permission::{PermissionGate, PermissionPolicy};
    use crate::provider::{
        CompletionResponse, ProviderConfig, ProviderFuture, ProviderKind, TokenUsage,
    };

    use super::*;

    struct MockProvider {
        config: ProviderConfig,
        responses: Mutex<VecDeque<CompletionResponse>>,
    }

    struct PendingProvider {
        config: ProviderConfig,
    }

    impl ModelProvider for PendingProvider {
        fn config(&self) -> &ProviderConfig {
            &self.config
        }

        fn complete(&self, _request: CompletionRequest) -> ProviderFuture<'_> {
            Box::pin(std::future::pending())
        }
    }

    impl ModelProvider for MockProvider {
        fn config(&self) -> &ProviderConfig {
            &self.config
        }

        fn complete(&self, _request: CompletionRequest) -> ProviderFuture<'_> {
            Box::pin(async move {
                Ok(self
                    .responses
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("mock response"))
            })
        }
    }

    fn response(content: &str, tool_calls: Vec<ToolCall>) -> CompletionResponse {
        CompletionResponse {
            content: content.to_owned(),
            model: Some("mock".into()),
            usage: TokenUsage::default(),
            tool_calls,
        }
    }

    fn engine(responses: Vec<CompletionResponse>) -> AgentEngine {
        let workspace = Arc::new(Workspace::open(env!("CARGO_MANIFEST_DIR")).unwrap());
        let permissions = Arc::new(PermissionGate::new(PermissionPolicy::default(), None));
        let provider = Arc::new(MockProvider {
            config: ProviderConfig::for_kind(ProviderKind::Local, "mock"),
            responses: Mutex::new(responses.into()),
        });
        AgentEngine::new(
            provider,
            Toolbox::new(workspace, permissions).unwrap(),
            Arc::new(NoopEventSink),
        )
    }

    #[tokio::test]
    async fn returns_provider_answer() {
        let result = engine(vec![response("complete", Vec::new())])
            .run("say complete")
            .await
            .unwrap();
        assert_eq!(result.final_answer, "complete");
        assert_eq!(result.steps, 1);
    }

    #[tokio::test]
    async fn executes_read_tool_and_continues() {
        let result = engine(vec![
            response(
                "",
                vec![ToolCall {
                    id: "call-1".into(),
                    name: "read_file".into(),
                    arguments: json!({"path": "Cargo.toml"}),
                }],
            ),
            response("manifest inspected", Vec::new()),
        ])
        .run("inspect the manifest")
        .await
        .unwrap();
        assert_eq!(result.final_answer, "manifest inspected");
        assert!(
            result
                .messages
                .iter()
                .any(|message| message.role == Role::Tool)
        );
    }

    #[tokio::test]
    async fn cancellation_interrupts_an_in_flight_model_request() {
        let workspace = Arc::new(Workspace::open(env!("CARGO_MANIFEST_DIR")).unwrap());
        let permissions = Arc::new(PermissionGate::new(PermissionPolicy::default(), None));
        let provider = Arc::new(PendingProvider {
            config: ProviderConfig::for_kind(ProviderKind::Local, "pending"),
        });
        let cancelled = Arc::new(AtomicBool::new(false));
        let engine = AgentEngine::new(
            provider,
            Toolbox::new(workspace, permissions).unwrap(),
            Arc::new(NoopEventSink),
        )
        .with_cancellation(cancelled.clone());
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            cancelled.store(true, Ordering::Relaxed);
        });

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            engine.run("wait for cancellation"),
        )
        .await
        .expect("cancellation should not wait for the provider");

        assert!(matches!(result, Err(AgentError::Cancelled)));
    }

    #[test]
    fn observable_edits_report_sizes_without_copying_file_contents() {
        let call = ToolCall {
            id: "call-1".into(),
            name: "write_file".into(),
            arguments: json!({"path": "note.txt", "content": "private body"}),
        };

        let observable = observable_tool_call(&call);

        assert_eq!(observable.arguments["path"], "note.txt");
        assert_eq!(observable.arguments["content_bytes"], 12);
        assert!(!observable.arguments.to_string().contains("private body"));
    }
}
