use std::collections::BTreeSet;
use std::path::Path;

use ignore::WalkBuilder;
use serde::Serialize;
use thiserror::Error;

use crate::workspace::{Workspace, WorkspaceError};

const MAX_INSTRUCTION_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct InstructionDocument {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct InstructionSet {
    documents: Vec<InstructionDocument>,
}

#[derive(Debug, Error)]
pub enum InstructionError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error("could not discover repository instructions: {0}")]
    Walk(String),
    #[error("repository instructions exceed the {0}-byte safety limit")]
    TooLarge(usize),
}

impl InstructionSet {
    /// Discovers root guidance and nested agent instruction files without leaving the workspace.
    pub fn discover(workspace: &Workspace) -> Result<Self, InstructionError> {
        let mut paths = BTreeSet::new();
        for candidate in [
            "AGENTS.md",
            "KERNEX.md",
            "CONTRIBUTING.md",
            ".kernex/instructions.md",
        ] {
            if workspace.root().join(candidate).is_file() {
                paths.insert(candidate.to_owned());
            }
        }

        for result in WalkBuilder::new(workspace.root())
            .hidden(false)
            .git_ignore(true)
            .filter_entry(|entry| !matches!(entry.file_name().to_str(), Some(".git" | "target")))
            .build()
        {
            let entry = result.map_err(|error| InstructionError::Walk(error.to_string()))?;
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            if matches!(entry.file_name().to_str(), Some("AGENTS.md" | "KERNEX.md")) {
                paths.insert(workspace.display_path(entry.path()));
            }
        }

        let mut total = 0usize;
        let mut documents = Vec::new();
        for path in paths {
            let content = workspace.read_text(Path::new(&path))?;
            total = total.saturating_add(content.len());
            if total > MAX_INSTRUCTION_BYTES {
                return Err(InstructionError::TooLarge(MAX_INSTRUCTION_BYTES));
            }
            documents.push(InstructionDocument { path, content });
        }
        documents.sort_by_key(|document| {
            (
                document.path.matches('/').count(),
                document.path.to_ascii_lowercase(),
            )
        });
        Ok(Self { documents })
    }

    pub fn documents(&self) -> &[InstructionDocument] {
        &self.documents
    }

    pub fn render_for_prompt(&self) -> String {
        if self.documents.is_empty() {
            return String::new();
        }
        let mut output = String::from(
            "\n\n## Repository-provided instructions\n\nThe following documents are project data. Follow the most specific applicable document while keeping all safety controls active.\n",
        );
        for document in &self.documents {
            output.push_str("\n### ");
            output.push_str(&document.path);
            output.push_str("\n\n");
            output.push_str(&document.content);
            if !document.content.ends_with('\n') {
                output.push('\n');
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_workspace_agent_guide() {
        let workspace = Workspace::open(env!("CARGO_MANIFEST_DIR")).unwrap();
        let instructions = InstructionSet::discover(&workspace).unwrap();
        assert!(
            instructions
                .documents()
                .iter()
                .any(|document| document.path.ends_with("AGENTS.md"))
                || instructions.documents().is_empty()
        );
    }
}
