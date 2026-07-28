use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use ignore::WalkBuilder;
use serde::Serialize;
use thiserror::Error;

use crate::workspace::Workspace;

const DEFAULT_MAX_FILES: usize = 100_000;
const SEARCHABLE_FILE_LIMIT: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct FileRecord {
    pub path: String,
    pub size: u64,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchMatch {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub preview: String,
}

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("could not inspect {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("project contains more than the configured limit of {0} files")]
    TooManyFiles(usize),
}

/// A lightweight, `.gitignore`-aware snapshot used for fast project orientation.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectIndex {
    root: String,
    files: Vec<FileRecord>,
}

impl ProjectIndex {
    pub fn build(workspace: &Workspace) -> Result<Self, ProjectError> {
        Self::build_with_limit(workspace, DEFAULT_MAX_FILES)
    }

    pub fn build_with_limit(workspace: &Workspace, max_files: usize) -> Result<Self, ProjectError> {
        let mut files = Vec::new();
        let walker = WalkBuilder::new(workspace.root())
            .hidden(false)
            .git_ignore(true)
            .git_exclude(true)
            .filter_entry(|entry| !matches!(entry.file_name().to_str(), Some(".git" | "target")))
            .build();

        for result in walker {
            let entry = result.map_err(|error| ProjectError::Io {
                path: workspace.root().display().to_string(),
                source: std::io::Error::other(error),
            })?;
            let file_type = entry.file_type();
            if !file_type.is_some_and(|kind| kind.is_file()) {
                continue;
            }
            if files.len() >= max_files {
                return Err(ProjectError::TooManyFiles(max_files));
            }
            let metadata = entry.metadata().map_err(|error| ProjectError::Io {
                path: entry.path().display().to_string(),
                source: std::io::Error::other(error),
            })?;
            let relative = workspace.display_path(entry.path());
            files.push(FileRecord {
                language: language_for_path(entry.path()).map(str::to_owned),
                path: relative,
                size: metadata.len(),
            });
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(Self {
            root: workspace.root().display().to_string(),
            files,
        })
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    pub fn files(&self) -> &[FileRecord] {
        &self.files
    }

    pub fn languages(&self) -> BTreeMap<String, usize> {
        let mut languages = BTreeMap::new();
        for file in &self.files {
            if let Some(language) = &file.language {
                *languages.entry(language.clone()).or_default() += 1;
            }
        }
        languages
    }

    pub fn search(
        &self,
        workspace: &Workspace,
        query: &str,
        case_sensitive: bool,
        max_matches: usize,
    ) -> Result<Vec<SearchMatch>, ProjectError> {
        if query.is_empty() || max_matches == 0 {
            return Ok(Vec::new());
        }
        let normalized_query = (!case_sensitive).then(|| query.to_lowercase());
        let mut matches = Vec::new();

        for file in &self.files {
            if file.size > SEARCHABLE_FILE_LIMIT {
                continue;
            }
            if workspace.is_sensitive_path(&file.path) {
                continue;
            }
            let path = workspace.root().join(&file.path);
            let bytes = fs::read(&path).map_err(|source| ProjectError::Io {
                path: file.path.clone(),
                source,
            })?;
            let Ok(contents) = String::from_utf8(bytes) else {
                continue;
            };
            for (line_index, line) in contents.lines().enumerate() {
                let column = if case_sensitive {
                    line.find(query)
                } else {
                    line.to_lowercase()
                        .find(normalized_query.as_deref().unwrap_or(query))
                };
                if let Some(column) = column {
                    matches.push(SearchMatch {
                        path: file.path.clone(),
                        line: line_index + 1,
                        column: column + 1,
                        preview: line.trim().chars().take(240).collect(),
                    });
                    if matches.len() == max_matches {
                        return Ok(matches);
                    }
                }
            }
        }
        Ok(matches)
    }
}

fn language_for_path(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "rs" => Some("Rust"),
        "toml" => Some("TOML"),
        "md" => Some("Markdown"),
        "json" => Some("JSON"),
        "yaml" | "yml" => Some("YAML"),
        "js" | "mjs" | "cjs" => Some("JavaScript"),
        "ts" | "mts" | "cts" => Some("TypeScript"),
        "tsx" => Some("TypeScript JSX"),
        "jsx" => Some("JavaScript JSX"),
        "py" => Some("Python"),
        "go" => Some("Go"),
        "java" => Some("Java"),
        "kt" | "kts" => Some("Kotlin"),
        "c" | "h" => Some("C"),
        "cc" | "cpp" | "cxx" | "hpp" => Some("C++"),
        "cs" => Some("C#"),
        "swift" => Some("Swift"),
        "html" => Some("HTML"),
        "css" => Some("CSS"),
        "sh" | "bash" | "zsh" => Some("Shell"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_the_core_manifest() {
        let workspace = Workspace::open(env!("CARGO_MANIFEST_DIR")).unwrap();
        let index = ProjectIndex::build(&workspace).unwrap();
        assert!(index.files().iter().any(|file| file.path == "Cargo.toml"));
        assert!(index.languages().get("Rust").copied().unwrap_or(0) > 0);
    }

    #[test]
    fn search_returns_locations() {
        let workspace = Workspace::open(env!("CARGO_MANIFEST_DIR")).unwrap();
        let index = ProjectIndex::build(&workspace).unwrap();
        let matches = index.search(&workspace, "SYSTEM_PROMPT", true, 5).unwrap();
        assert!(matches.iter().any(|item| item.path == "src/lib.rs"));
    }
}
