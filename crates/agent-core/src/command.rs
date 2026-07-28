use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

use crate::permission::{
    Capability, PermissionError, PermissionGate, PermissionRequest, RiskLevel,
};
use crate::workspace::{Workspace, WorkspaceError};

const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
const MAX_CAPTURE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSpec {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
}

impl CommandSpec {
    pub fn new(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            cwd: None,
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            stdin: None,
        }
    }

    pub fn display(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(quote_argument)
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn risk(&self) -> RiskLevel {
        classify_risk(&self.program, &self.args)
    }
}

fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT_SECONDS
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandOutput {
    pub command: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

#[derive(Debug, Error)]
pub enum CommandError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Permission(#[from] PermissionError),
    #[error("command program cannot be empty")]
    EmptyProgram,
    #[error("failed to start {command}: {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write input to {command}: {source}")]
    Stdin {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("command timed out after {seconds} seconds: {command}")]
    TimedOut { command: String, seconds: u64 },
}

/// Executes argument-vector commands without shell expansion and inside the workspace boundary.
pub struct CommandRunner {
    workspace: Arc<Workspace>,
    permissions: Arc<PermissionGate>,
}

impl CommandRunner {
    pub fn new(workspace: Arc<Workspace>, permissions: Arc<PermissionGate>) -> Self {
        Self {
            workspace,
            permissions,
        }
    }

    pub async fn run(&self, spec: &CommandSpec) -> Result<CommandOutput, CommandError> {
        if spec.program.trim().is_empty() {
            return Err(CommandError::EmptyProgram);
        }
        let rendered = spec.display();
        let cwd = match &spec.cwd {
            Some(path) => self.workspace.resolve_existing(path)?,
            None => self.workspace.root().to_path_buf(),
        };
        self.permissions.authorize(&PermissionRequest {
            capability: Capability::ExecuteCommand,
            risk: spec.risk(),
            summary: format!("run `{rendered}`"),
            resource: self.workspace.display_path(&cwd),
            details: vec![format!("working directory: {}", cwd.display())],
        })?;

        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .current_dir(cwd)
            .env_clear()
            .envs(safe_environment())
            .stdin(if spec.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = command.spawn().map_err(|source| CommandError::Spawn {
            command: rendered.clone(),
            source,
        })?;
        let input = spec.stdin.clone();
        let operation_command = rendered.clone();
        let operation = async move {
            if let Some(input) = input {
                let mut stdin = child.stdin.take().ok_or_else(|| CommandError::Stdin {
                    command: operation_command.clone(),
                    source: std::io::Error::other("child process did not expose stdin"),
                })?;
                stdin
                    .write_all(input.as_bytes())
                    .await
                    .map_err(|source| CommandError::Stdin {
                        command: operation_command.clone(),
                        source,
                    })?;
                stdin
                    .shutdown()
                    .await
                    .map_err(|source| CommandError::Stdin {
                        command: operation_command.clone(),
                        source,
                    })?;
            }
            child
                .wait_with_output()
                .await
                .map_err(|source| CommandError::Spawn {
                    command: operation_command,
                    source,
                })
        };
        let output = timeout(Duration::from_secs(spec.timeout_seconds.max(1)), operation)
            .await
            .map_err(|_| CommandError::TimedOut {
                command: rendered.clone(),
                seconds: spec.timeout_seconds.max(1),
            })??;

        let (stdout, stdout_truncated) = bounded_text(&output.stdout);
        let (stderr, stderr_truncated) = bounded_text(&output.stderr);
        Ok(CommandOutput {
            command: rendered,
            exit_code: output.status.code(),
            success: output.status.success(),
            stdout,
            stderr,
            truncated: stdout_truncated || stderr_truncated,
        })
    }
}

fn classify_risk(program: &str, args: &[String]) -> RiskLevel {
    let name = Path::new(program)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(program)
        .to_ascii_lowercase();
    let normalized_args: Vec<_> = args.iter().map(|arg| arg.to_ascii_lowercase()).collect();

    if matches!(
        name.as_str(),
        "rm" | "rmdir" | "del" | "erase" | "mkfs" | "format" | "shutdown" | "reboot"
    ) {
        return RiskLevel::Critical;
    }
    if matches!(name.as_str(), "sudo" | "doas" | "su") {
        return RiskLevel::Critical;
    }
    if name == "git"
        && normalized_args.iter().any(|arg| {
            matches!(
                arg.as_str(),
                "push" | "reset" | "clean" | "rebase" | "commit" | "merge"
            )
        })
    {
        return RiskLevel::High;
    }
    if matches!(
        name.as_str(),
        "sh" | "bash" | "zsh" | "fish" | "cmd" | "powershell" | "pwsh"
    ) {
        return RiskLevel::High;
    }
    if matches!(name.as_str(), "cargo" | "npm" | "pnpm" | "yarn" | "make") {
        return RiskLevel::Medium;
    }
    RiskLevel::Low
}

fn quote_argument(argument: &str) -> String {
    if argument
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "-_./:=@".contains(character))
    {
        argument.to_owned()
    } else {
        format!("{:?}", argument)
    }
}

fn bounded_text(bytes: &[u8]) -> (String, bool) {
    let truncated = bytes.len() > MAX_CAPTURE_BYTES;
    let selected = &bytes[..bytes.len().min(MAX_CAPTURE_BYTES)];
    (String::from_utf8_lossy(selected).into_owned(), truncated)
}

pub(crate) fn safe_environment() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(name, _)| !is_sensitive_environment_name(name))
        .collect()
}

fn is_sensitive_environment_name(name: &OsStr) -> bool {
    let name = name.to_string_lossy().to_ascii_uppercase();
    [
        "KEY",
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "AUTH",
        "COOKIE",
        "SESSION",
        "PRIVATE",
        "PROXY",
    ]
    .iter()
    .any(|marker| name.contains(marker))
        || name == "DATABASE_URL"
        || name == "GOOGLE_APPLICATION_CREDENTIALS"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destructive_commands_are_critical() {
        assert_eq!(
            CommandSpec::new("rm", ["-rf", "build"]).risk(),
            RiskLevel::Critical
        );
    }

    #[test]
    fn shell_commands_are_high_risk() {
        assert_eq!(
            CommandSpec::new("sh", ["-c", "echo hello"]).risk(),
            RiskLevel::High
        );
    }

    #[test]
    fn display_quotes_whitespace_without_using_a_shell() {
        let command = CommandSpec::new("printf", ["hello world"]);
        assert_eq!(command.display(), "printf \"hello world\"");
    }

    #[test]
    fn sensitive_environment_names_are_filtered() {
        assert!(is_sensitive_environment_name(OsStr::new("OPENAI_API_KEY")));
        assert!(is_sensitive_environment_name(OsStr::new("SSH_AUTH_SOCK")));
        assert!(!is_sensitive_environment_name(OsStr::new("PATH")));
        assert!(!is_sensitive_environment_name(OsStr::new("CARGO_HOME")));
    }
}
