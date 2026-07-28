use std::process::Command;
use std::sync::Arc;

use serde::Serialize;
use thiserror::Error;

use crate::diff::unified_diff;
use crate::permission::{
    Capability, PermissionError, PermissionGate, PermissionRequest, RiskLevel,
};
use crate::workspace::Workspace;

#[derive(Debug, Clone, Serialize)]
pub struct GitOutput {
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Error)]
pub enum GitError {
    #[error(transparent)]
    Permission(#[from] PermissionError),
    #[error("failed to start Git: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("Git command failed: {0}")]
    Failed(String),
}

/// Read-only Git facade; mutating Git operations stay behind the general command permission path.
pub struct GitRepository {
    workspace: Arc<Workspace>,
    permissions: Arc<PermissionGate>,
}

impl GitRepository {
    pub fn new(workspace: Arc<Workspace>, permissions: Arc<PermissionGate>) -> Self {
        Self {
            workspace,
            permissions,
        }
    }

    pub fn status(&self) -> Result<GitOutput, GitError> {
        self.run_read(
            "inspect repository status",
            &["status", "--short", "--branch"],
        )
    }

    pub fn diff(&self, staged: bool) -> Result<GitOutput, GitError> {
        if staged {
            let mut output = self.run_read(
                "review staged changes",
                &["diff", "--cached", "--no-ext-diff"],
            )?;
            output.stdout = redact_sensitive_diffs(&self.workspace, &output.stdout);
            Ok(output)
        } else {
            let mut output =
                self.run_read("review working tree changes", &["diff", "--no-ext-diff"])?;
            output.stdout = redact_sensitive_diffs(&self.workspace, &output.stdout);
            let untracked = self.run_read(
                "list untracked changes",
                &["ls-files", "--others", "--exclude-standard", "-z"],
            )?;
            for path in untracked.stdout.split('\0').filter(|path| !path.is_empty()) {
                if self.workspace.is_sensitive_path(path) {
                    output.stdout.push_str(&format!(
                        "--- /dev/null\n+++ b/{path}\n@@ sensitive @@\n+[sensitive file content withheld]\n"
                    ));
                    continue;
                }
                output
                    .stdout
                    .push_str(&match self.workspace.read_text(path) {
                        Ok(contents) => unified_diff(path, "", &contents),
                        Err(error) => {
                            format!("--- /dev/null\n+++ b/{path}\n@@ unreadable @@\n+[{error}]\n")
                        }
                    });
            }
            Ok(output)
        }
    }

    pub fn tracked_files(&self) -> Result<Vec<String>, GitError> {
        let output = self.run_read("list tracked files", &["ls-files", "-z"])?;
        Ok(output
            .stdout
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .collect())
    }

    pub fn log(&self, max_entries: usize) -> Result<GitOutput, GitError> {
        let max_entries = max_entries.clamp(1, 200).to_string();
        self.run_read(
            "inspect repository history",
            &[
                "log",
                "--date=short",
                "--pretty=format:%h%x09%ad%x09%an%x09%s",
                "-n",
                &max_entries,
            ],
        )
    }

    fn run_read(&self, summary: &str, args: &[&str]) -> Result<GitOutput, GitError> {
        self.permissions.authorize(&PermissionRequest {
            capability: Capability::GitRead,
            risk: RiskLevel::Low,
            summary: summary.to_owned(),
            resource: self.workspace.root().display().to_string(),
            details: vec![format!("git {}", args.join(" "))],
        })?;

        let output = Command::new("git")
            .arg("-C")
            .arg(self.workspace.root())
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .map_err(GitError::Spawn)?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.status.success() {
            return Err(GitError::Failed(stderr.trim().to_owned()));
        }
        Ok(GitOutput { stdout, stderr })
    }
}

fn redact_sensitive_diffs(workspace: &Workspace, diff: &str) -> String {
    let mut output = String::new();
    let mut section = String::new();
    let mut sensitive = false;
    for line in diff.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            flush_diff_section(&mut output, &section, sensitive);
            section.clear();
            sensitive = line
                .split_whitespace()
                .skip(2)
                .map(|path| {
                    path.trim_matches('"')
                        .trim_start_matches("a/")
                        .trim_start_matches("b/")
                })
                .any(|path| workspace.is_sensitive_path(path));
        }
        section.push_str(line);
    }
    flush_diff_section(&mut output, &section, sensitive);
    output
}

fn flush_diff_section(output: &mut String, section: &str, sensitive: bool) {
    if section.is_empty() {
        return;
    }
    if sensitive {
        if let Some(header) = section.lines().next() {
            output.push_str(header);
            output.push_str("\n[sensitive diff content withheld]\n");
        }
    } else {
        output.push_str(section);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_sensitive_diff_sections() {
        let workspace = Workspace::open(env!("CARGO_MANIFEST_DIR")).unwrap();
        let diff = "diff --git a/.env b/.env\n--- a/.env\n+++ b/.env\n+TOKEN=value\ndiff --git a/src/lib.rs b/src/lib.rs\n+safe\n";
        let redacted = redact_sensitive_diffs(&workspace, diff);
        assert!(!redacted.contains("TOKEN=value"));
        assert!(redacted.contains("sensitive diff content withheld"));
        assert!(redacted.contains("+safe"));
    }
}
