use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

use crate::permission::{
    Capability, PermissionError, PermissionGate, PermissionRequest, RiskLevel,
};
use crate::workspace::{Workspace, WorkspaceError};

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub before: String,
    pub after: String,
    pub diff: String,
}

#[derive(Debug, Error)]
pub enum FileEditError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Permission(#[from] PermissionError),
    #[error("failed to write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("text to replace was not found in {path}")]
    TextNotFound { path: String },
    #[error("cannot create {path} because it already exists")]
    AlreadyExists { path: String },
    #[error(
        "text to replace occurs {count} times in {path}; use replace_all or provide more context"
    )]
    Ambiguous { path: String, count: usize },
}

/// Permissioned text editor that returns a reviewable diff for every mutation.
pub struct FileEditor {
    workspace: Arc<Workspace>,
    permissions: Arc<PermissionGate>,
}

impl FileEditor {
    pub fn new(workspace: Arc<Workspace>, permissions: Arc<PermissionGate>) -> Self {
        Self {
            workspace,
            permissions,
        }
    }

    pub fn write_text(
        &self,
        path: impl AsRef<Path>,
        contents: &str,
    ) -> Result<FileChange, FileEditError> {
        let resolved = self.workspace.resolve_for_write(path.as_ref())?;
        let before = if resolved.exists() {
            self.workspace.read_text(&resolved)?
        } else {
            String::new()
        };
        let display_path = self.workspace.display_path(&resolved);
        let diff = unified_diff(&display_path, &before, contents);

        self.permissions.authorize(&PermissionRequest {
            capability: Capability::WriteFile,
            risk: RiskLevel::Medium,
            summary: format!("write {display_path}"),
            resource: display_path.clone(),
            details: vec![diff.clone()],
        })?;

        atomic_write(&resolved, contents.as_bytes()).map_err(|source| FileEditError::Write {
            path: display_path.clone(),
            source,
        })?;

        Ok(FileChange {
            path: display_path,
            before,
            after: contents.to_owned(),
            diff,
        })
    }

    pub fn create_text(
        &self,
        path: impl AsRef<Path>,
        contents: &str,
    ) -> Result<FileChange, FileEditError> {
        let resolved = self.workspace.resolve_for_write(path.as_ref())?;
        if resolved.exists() {
            return Err(FileEditError::AlreadyExists {
                path: self.workspace.display_path(resolved),
            });
        }
        self.write_text(path, contents)
    }

    pub fn replace_text(
        &self,
        path: impl AsRef<Path>,
        old: &str,
        new: &str,
        replace_all: bool,
    ) -> Result<FileChange, FileEditError> {
        let resolved = self.workspace.resolve_existing(path.as_ref())?;
        let display_path = self.workspace.display_path(&resolved);
        let before = self.workspace.read_text(&resolved)?;
        let count = before.matches(old).count();
        if count == 0 {
            return Err(FileEditError::TextNotFound { path: display_path });
        }
        if count > 1 && !replace_all {
            return Err(FileEditError::Ambiguous {
                path: display_path,
                count,
            });
        }
        let after = if replace_all {
            before.replace(old, new)
        } else {
            before.replacen(old, new, 1)
        };
        self.write_text(resolved, &after)
    }
}

fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("target path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let (temporary_path, mut temporary) = create_temporary_file(parent, file_name)?;

    let result = (|| {
        temporary.write_all(contents)?;
        if let Ok(metadata) = fs::metadata(path) {
            fs::set_permissions(&temporary_path, metadata.permissions())?;
        }
        temporary.sync_all()?;
        drop(temporary);
        replace_file(&temporary_path, path)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn create_temporary_file(parent: &Path, file_name: &str) -> std::io::Result<(PathBuf, File)> {
    for _ in 0..128 {
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".{file_name}.kernex-{}-{sequence}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a unique temporary file",
    ))
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: both pointers reference valid, NUL-terminated UTF-16 buffers for this call.
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Produces a deterministic unified-style diff without invoking a shell command.
pub fn unified_diff(path: &str, before: &str, after: &str) -> String {
    if before == after {
        return format!("--- a/{path}\n+++ b/{path}\n");
    }

    let old_lines: Vec<_> = before.lines().collect();
    let new_lines: Vec<_> = after.lines().collect();
    let operations = lcs_operations(&old_lines, &new_lines);
    let mut output = format!(
        "--- a/{path}\n+++ b/{path}\n@@ -1,{} +1,{} @@\n",
        old_lines.len(),
        new_lines.len()
    );
    for (prefix, line) in operations {
        output.push(prefix);
        output.push_str(line);
        output.push('\n');
    }
    if before.ends_with('\n') != after.ends_with('\n') {
        output.push_str("\\ No newline at end of file\n");
    }
    output
}

fn lcs_operations<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(char, &'a str)> {
    const MAX_MATRIX_CELLS: usize = 1_000_000;
    if old.len().saturating_mul(new.len()) > MAX_MATRIX_CELLS {
        return old
            .iter()
            .map(|line| ('-', *line))
            .chain(new.iter().map(|line| ('+', *line)))
            .collect();
    }

    let mut table = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for old_index in (0..old.len()).rev() {
        for new_index in (0..new.len()).rev() {
            table[old_index][new_index] = if old[old_index] == new[new_index] {
                table[old_index + 1][new_index + 1] + 1
            } else {
                table[old_index + 1][new_index].max(table[old_index][new_index + 1])
            };
        }
    }

    let mut operations = Vec::new();
    let (mut old_index, mut new_index) = (0, 0);
    while old_index < old.len() && new_index < new.len() {
        if old[old_index] == new[new_index] {
            operations.push((' ', old[old_index]));
            old_index += 1;
            new_index += 1;
        } else if table[old_index + 1][new_index] >= table[old_index][new_index + 1] {
            operations.push(('-', old[old_index]));
            old_index += 1;
        } else {
            operations.push(('+', new[new_index]));
            new_index += 1;
        }
    }
    operations.extend(old[old_index..].iter().map(|line| ('-', *line)));
    operations.extend(new[new_index..].iter().map(|line| ('+', *line)));
    operations
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    fn temporary_directory() -> PathBuf {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "kernex-atomic-write-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn diff_marks_changed_lines() {
        let diff = unified_diff("src/lib.rs", "one\ntwo\n", "one\nthree\n");
        assert!(diff.contains("-two"));
        assert!(diff.contains("+three"));
        assert!(diff.contains(" one"));
    }

    #[test]
    fn unchanged_diff_has_headers_only() {
        assert_eq!(
            unified_diff("file.txt", "same", "same"),
            "--- a/file.txt\n+++ b/file.txt\n"
        );
    }

    #[test]
    fn atomic_write_replaces_contents_without_leaving_temporary_files() {
        let directory = temporary_directory();
        let path = directory.join("note.txt");
        fs::write(&path, "before").unwrap();

        atomic_write(&path, b"after").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "after");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_preserves_unix_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = temporary_directory();
        let path = directory.join("script.sh");
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();

        atomic_write(&path, b"new").unwrap();

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o751
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
