use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{SecondsFormat, Utc};
use directories::ProjectDirs;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::agent::{AgentEvent, AgentRunResult, EventSink};
use crate::permission::{Approver, Capability, PermissionDecision, PermissionRequest, RiskLevel};
use crate::provider::{Message, Role, TokenUsage, ToolCall};

const DATABASE_FILE: &str = "sessions.sqlite3";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultRecord {
    pub call_id: String,
    pub name: String,
    pub result: String,
    pub error: Option<String>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionAuditRecord {
    pub capability: Capability,
    pub risk: RiskLevel,
    pub resource: String,
    pub summary: String,
    pub decision: PermissionDecision,
    pub timestamp: String,
}

/// Complete session data shared by the CLI and desktop application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub workspace_path: String,
    pub provider: String,
    pub model: String,
    pub messages: Vec<Message>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_results: Vec<ToolResultRecord>,
    pub permission_decisions: Vec<PermissionAuditRecord>,
    pub created_at: String,
    pub updated_at: String,
    pub token_usage: TokenUsage,
    pub generated_diffs: Vec<String>,
    pub status: SessionStatus,
}

impl SessionRecord {
    pub fn new(
        workspace_path: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        let timestamp = now();
        Self {
            id: Uuid::new_v4().to_string(),
            workspace_path: workspace_path.into(),
            provider: provider.into(),
            model: model.into(),
            messages: Vec::new(),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
            permission_decisions: Vec::new(),
            created_at: timestamp.clone(),
            updated_at: timestamp,
            token_usage: TokenUsage::default(),
            generated_diffs: Vec::new(),
            status: SessionStatus::Active,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = now();
    }
}

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("Kernex could not determine a local data directory")]
    MissingDataDirectory,
    #[error("could not prepare the session directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("session database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("session data could not be serialized: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("session storage is unavailable")]
    Unavailable,
}

/// SQLite-backed session repository used by every Kernex user interface.
pub struct SessionStore {
    connection: Mutex<Connection>,
    path: PathBuf,
}

/// Event sink that durably records a session while forwarding events to its UI.
pub struct SessionRecorder {
    store: Arc<SessionStore>,
    session: Arc<Mutex<SessionRecord>>,
    downstream: Arc<dyn EventSink>,
}

impl SessionRecorder {
    pub fn new(
        store: Arc<SessionStore>,
        session: SessionRecord,
        downstream: Arc<dyn EventSink>,
    ) -> Result<Self, SessionError> {
        let recorder = Self {
            store,
            session: Arc::new(Mutex::new(session)),
            downstream,
        };
        recorder.persist()?;
        Ok(recorder)
    }

    pub fn snapshot(&self) -> Result<SessionRecord, SessionError> {
        self.session
            .lock()
            .map(|session| session.clone())
            .map_err(|_| SessionError::Unavailable)
    }

    pub fn complete(&self, result: &AgentRunResult) -> Result<(), SessionError> {
        let mut session = self.session.lock().map_err(|_| SessionError::Unavailable)?;
        session.messages.clone_from(&result.messages);
        session.token_usage.clone_from(&result.token_usage);
        session.status = SessionStatus::Completed;
        self.store.save(&mut session)
    }

    pub fn fail(&self, cancelled: bool) -> Result<(), SessionError> {
        let mut session = self.session.lock().map_err(|_| SessionError::Unavailable)?;
        session.status = if cancelled {
            SessionStatus::Cancelled
        } else {
            SessionStatus::Failed
        };
        self.store.save(&mut session)
    }

    pub fn record_permission(
        &self,
        request: &PermissionRequest,
        decision: PermissionDecision,
    ) -> Result<(), SessionError> {
        let mut session = self.session.lock().map_err(|_| SessionError::Unavailable)?;
        session.permission_decisions.push(PermissionAuditRecord {
            capability: request.capability,
            risk: request.risk,
            resource: request.resource.clone(),
            summary: request.summary.clone(),
            decision,
            timestamp: now(),
        });
        self.store.save(&mut session)
    }

    fn persist(&self) -> Result<(), SessionError> {
        let mut session = self.session.lock().map_err(|_| SessionError::Unavailable)?;
        self.store.save(&mut session)
    }

    fn record_event(&self, event: &AgentEvent) -> Result<(), SessionError> {
        let mut session = self.session.lock().map_err(|_| SessionError::Unavailable)?;
        match event {
            AgentEvent::Started { task, .. } => {
                session.status = SessionStatus::Active;
                if !session
                    .messages
                    .last()
                    .is_some_and(|message| message.role == Role::User && message.content == *task)
                {
                    session.messages.push(Message::new(Role::User, task));
                }
            }
            AgentEvent::ModelResponded {
                content,
                tool_calls,
                ..
            } => {
                let mut message = Message::new(Role::Assistant, content);
                message.tool_calls.clone_from(tool_calls);
                session.messages.push(message);
                session.tool_calls.extend(tool_calls.iter().cloned());
            }
            AgentEvent::ToolFinished {
                call_id,
                name,
                result,
                diff,
                ..
            } => {
                session.tool_results.push(ToolResultRecord {
                    call_id: call_id.clone(),
                    name: name.clone(),
                    result: result.clone(),
                    error: None,
                    timestamp: now(),
                });
                session.messages.push(Message {
                    role: Role::Tool,
                    content: result.clone(),
                    name: Some(name.clone()),
                    tool_call_id: Some(call_id.clone()),
                    tool_calls: Vec::new(),
                });
                if let Some(diff) = diff {
                    session.generated_diffs.push(diff.clone());
                }
            }
            AgentEvent::ToolFailed {
                call_id,
                name,
                error,
                ..
            } => session.tool_results.push(ToolResultRecord {
                call_id: call_id.clone(),
                name: name.clone(),
                result: String::new(),
                error: Some(error.clone()),
                timestamp: now(),
            }),
            AgentEvent::Completed { .. } => session.status = SessionStatus::Completed,
            AgentEvent::ModelRequested { .. }
            | AgentEvent::ModelDelta { .. }
            | AgentEvent::ToolStarted { .. } => return Ok(()),
        }
        self.store.save(&mut session)
    }
}

impl EventSink for SessionRecorder {
    fn emit(&self, event: AgentEvent) {
        let _ = self.record_event(&event);
        self.downstream.emit(event);
    }
}

/// Approver decorator that stores every user decision in the active session.
pub struct AuditedApprover {
    inner: Arc<dyn Approver>,
    recorder: Arc<SessionRecorder>,
}

impl AuditedApprover {
    pub fn new(inner: Arc<dyn Approver>, recorder: Arc<SessionRecorder>) -> Self {
        Self { inner, recorder }
    }
}

impl Approver for AuditedApprover {
    fn decide(&self, request: &PermissionRequest) -> PermissionDecision {
        let decision = self.inner.decide(request);
        let _ = self.recorder.record_permission(request, decision);
        decision
    }
}

impl SessionStore {
    pub fn open_default() -> Result<Self, SessionError> {
        let directories = ProjectDirs::from("dev", "Kernex", "Kernex")
            .ok_or(SessionError::MissingDataDirectory)?;
        Self::open(directories.data_local_dir().join(DATABASE_FILE))
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, SessionError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| SessionError::CreateDirectory {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let connection = Connection::open(&path)?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS sessions (
                 id TEXT PRIMARY KEY NOT NULL,
                 workspace_path TEXT NOT NULL,
                 provider TEXT NOT NULL,
                 model TEXT NOT NULL,
                 status TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL,
                 data_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS sessions_workspace_updated
                 ON sessions(workspace_path, updated_at DESC);",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
            path,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn save(&self, session: &mut SessionRecord) -> Result<(), SessionError> {
        session.touch();
        let data = serde_json::to_string(session)?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| SessionError::Unavailable)?;
        connection.execute(
            "INSERT INTO sessions (
                id, workspace_path, provider, model, status, created_at, updated_at, data_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                workspace_path = excluded.workspace_path,
                provider = excluded.provider,
                model = excluded.model,
                status = excluded.status,
                updated_at = excluded.updated_at,
                data_json = excluded.data_json",
            params![
                session.id,
                session.workspace_path,
                session.provider,
                session.model,
                status_name(session.status),
                session.created_at,
                session.updated_at,
                data,
            ],
        )?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Option<SessionRecord>, SessionError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| SessionError::Unavailable)?;
        let data = connection
            .query_row(
                "SELECT data_json FROM sessions WHERE id = ?1",
                [id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        data.map(|value| serde_json::from_str(&value).map_err(SessionError::from))
            .transpose()
    }

    pub fn list(&self, limit: usize) -> Result<Vec<SessionRecord>, SessionError> {
        self.query_sessions(
            "SELECT data_json FROM sessions ORDER BY updated_at DESC LIMIT ?1",
            &[&(limit.min(1_000) as i64)],
        )
    }

    pub fn list_for_workspace(
        &self,
        workspace_path: &str,
        limit: usize,
    ) -> Result<Vec<SessionRecord>, SessionError> {
        self.query_sessions(
            "SELECT data_json FROM sessions
             WHERE workspace_path = ?1 ORDER BY updated_at DESC LIMIT ?2",
            &[
                &workspace_path as &dyn rusqlite::ToSql,
                &(limit.min(1_000) as i64),
            ],
        )
    }

    pub fn latest_for_workspace(
        &self,
        workspace_path: &str,
    ) -> Result<Option<SessionRecord>, SessionError> {
        Ok(self
            .list_for_workspace(workspace_path, 1)?
            .into_iter()
            .next())
    }

    pub fn delete(&self, id: &str) -> Result<bool, SessionError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| SessionError::Unavailable)?;
        Ok(connection.execute("DELETE FROM sessions WHERE id = ?1", [id])? > 0)
    }

    fn query_sessions(
        &self,
        query: &str,
        parameters: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<SessionRecord>, SessionError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| SessionError::Unavailable)?;
        let mut statement = connection.prepare(query)?;
        let values = statement
            .query_map(parameters, |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        values
            .into_iter()
            .map(|value| serde_json::from_str(&value).map_err(SessionError::from))
            .collect()
    }
}

fn status_name(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Active => "active",
        SessionStatus::Completed => "completed",
        SessionStatus::Cancelled => "cancelled",
        SessionStatus::Failed => "failed",
    }
}

pub(crate) fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::NoopEventSink;
    use crate::provider::{Message, Role};

    fn temporary_database() -> PathBuf {
        std::env::temp_dir().join(format!(
            "kernex-session-test-{}-{}.sqlite3",
            std::process::id(),
            Uuid::new_v4()
        ))
    }

    #[test]
    fn sessions_round_trip_through_sqlite() {
        let path = temporary_database();
        let store = SessionStore::open(&path).unwrap();
        let mut session = SessionRecord::new("/workspace", "local", "test-model");
        session.messages.push(Message::new(Role::User, "hello"));
        store.save(&mut session).unwrap();

        let restored = store.get(&session.id).unwrap().unwrap();
        assert_eq!(restored.workspace_path, "/workspace");
        assert_eq!(restored.messages[0].content, "hello");
        assert_eq!(
            store
                .latest_for_workspace("/workspace")
                .unwrap()
                .unwrap()
                .id,
            session.id
        );

        drop(store);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn sessions_are_isolated_by_workspace() {
        let path = temporary_database();
        let store = SessionStore::open(&path).unwrap();
        let mut first = SessionRecord::new("/first", "local", "one");
        let mut second = SessionRecord::new("/second", "local", "two");
        store.save(&mut first).unwrap();
        store.save(&mut second).unwrap();

        let sessions = store.list_for_workspace("/first", 10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, first.id);

        drop(store);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn cancelled_sessions_retain_the_started_user_turn() {
        let path = temporary_database();
        let store = Arc::new(SessionStore::open(&path).unwrap());
        let session = SessionRecord::new("/workspace", "local", "test-model");
        let id = session.id.clone();
        let recorder =
            SessionRecorder::new(store.clone(), session, Arc::new(NoopEventSink)).unwrap();
        recorder.emit(AgentEvent::Started {
            task: "keep this request".into(),
            provider: "local".into(),
            model: "test-model".into(),
        });
        recorder.fail(true).unwrap();

        let restored = store.get(&id).unwrap().unwrap();
        assert_eq!(restored.status, SessionStatus::Cancelled);
        assert_eq!(restored.messages[0].role, Role::User);
        assert_eq!(restored.messages[0].content, "keep this request");

        drop(recorder);
        drop(store);
        fs::remove_file(path).unwrap();
    }
}
