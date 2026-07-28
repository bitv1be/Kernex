use std::fs;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

const MAX_TEXT_FILE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("workspace does not exist: {0}")]
    Missing(PathBuf),
    #[error("workspace path is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("path escapes the workspace: {0}")]
    OutsideWorkspace(PathBuf),
    #[error("path must not contain parent traversal: {0}")]
    ParentTraversal(PathBuf),
    #[error("file is too large to read safely ({size} bytes): {path}")]
    FileTooLarge { path: PathBuf, size: u64 },
    #[error("file is not valid UTF-8: {0}")]
    NotUtf8(PathBuf),
    #[error("I/O error for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Canonical repository boundary used by all filesystem and command tools.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        let requested = path.as_ref();
        if !requested.exists() {
            return Err(WorkspaceError::Missing(requested.to_path_buf()));
        }
        let root = fs::canonicalize(requested).map_err(|source| WorkspaceError::Io {
            path: requested.to_path_buf(),
            source,
        })?;
        if !root.is_dir() {
            return Err(WorkspaceError::NotDirectory(root));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn resolve_existing(&self, path: impl AsRef<Path>) -> Result<PathBuf, WorkspaceError> {
        let candidate = self.lexical_candidate(path.as_ref())?;
        let resolved = fs::canonicalize(&candidate).map_err(|source| WorkspaceError::Io {
            path: candidate.clone(),
            source,
        })?;
        self.ensure_inside(resolved)
    }

    pub fn resolve_for_write(&self, path: impl AsRef<Path>) -> Result<PathBuf, WorkspaceError> {
        let candidate = self.lexical_candidate(path.as_ref())?;
        if candidate.exists() {
            return self.resolve_existing(candidate);
        }

        let Some(parent) = candidate.parent() else {
            return Err(WorkspaceError::OutsideWorkspace(candidate));
        };
        let resolved_parent = fs::canonicalize(parent).map_err(|source| WorkspaceError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
        self.ensure_inside(resolved_parent)?;
        Ok(candidate)
    }

    pub fn display_path(&self, path: impl AsRef<Path>) -> String {
        let display = path
            .as_ref()
            .strip_prefix(&self.root)
            .unwrap_or(path.as_ref())
            .to_string_lossy()
            .into_owned();
        if display.is_empty() {
            ".".to_owned()
        } else {
            display
        }
    }

    pub fn is_sensitive_path(&self, path: impl AsRef<Path>) -> bool {
        is_sensitive_path(path.as_ref())
    }

    pub fn read_text(&self, path: impl AsRef<Path>) -> Result<String, WorkspaceError> {
        let resolved = self.resolve_existing(path)?;
        let metadata = fs::metadata(&resolved).map_err(|source| WorkspaceError::Io {
            path: resolved.clone(),
            source,
        })?;
        if metadata.len() > MAX_TEXT_FILE_BYTES {
            return Err(WorkspaceError::FileTooLarge {
                path: resolved,
                size: metadata.len(),
            });
        }
        let bytes = fs::read(&resolved).map_err(|source| WorkspaceError::Io {
            path: resolved.clone(),
            source,
        })?;
        String::from_utf8(bytes).map_err(|_| WorkspaceError::NotUtf8(resolved))
    }

    fn lexical_candidate(&self, path: &Path) -> Result<PathBuf, WorkspaceError> {
        if path.components().any(|part| part == Component::ParentDir) {
            return Err(WorkspaceError::ParentTraversal(path.to_path_buf()));
        }
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        if !candidate.starts_with(&self.root) {
            return Err(WorkspaceError::OutsideWorkspace(candidate));
        }
        Ok(candidate)
    }

    fn ensure_inside(&self, path: PathBuf) -> Result<PathBuf, WorkspaceError> {
        if path.starts_with(&self.root) {
            Ok(path)
        } else {
            Err(WorkspaceError::OutsideWorkspace(path))
        }
    }
}

pub(crate) fn is_sensitive_path(path: &Path) -> bool {
    let components: Vec<_> = path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .map(str::to_ascii_lowercase)
        .collect();
    if components
        .iter()
        .any(|component| matches!(component.as_str(), ".ssh" | ".aws" | ".gnupg"))
    {
        return true;
    }
    let Some(file_name) = components.last() else {
        return false;
    };
    file_name == ".env"
        || file_name.starts_with(".env.")
        || matches!(
            file_name.as_str(),
            ".npmrc"
                | ".pypirc"
                | ".netrc"
                | "credentials"
                | "credentials.json"
                | "secrets.json"
                | "google-services.json"
                | "id_rsa"
                | "id_dsa"
                | "id_ecdsa"
                | "id_ed25519"
        )
        || Path::new(file_name)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension,
                    "pem" | "key" | "p12" | "pfx" | "jks" | "keystore"
                )
            })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_traversal() {
        let workspace = Workspace::open(env!("CARGO_MANIFEST_DIR")).unwrap();
        assert!(matches!(
            workspace.resolve_existing("../Cargo.toml"),
            Err(WorkspaceError::ParentTraversal(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_absolute_path_outside_workspace() {
        let workspace = Workspace::open(env!("CARGO_MANIFEST_DIR")).unwrap();
        assert!(matches!(
            workspace.resolve_existing("/etc/passwd"),
            Err(WorkspaceError::OutsideWorkspace(_))
        ));
    }

    #[test]
    fn identifies_common_secret_paths() {
        assert!(is_sensitive_path(Path::new(".env.local")));
        assert!(is_sensitive_path(Path::new("config/signing.key")));
        assert!(is_sensitive_path(Path::new(".aws/credentials")));
        assert!(!is_sensitive_path(Path::new("src/provider.rs")));
    }
}
