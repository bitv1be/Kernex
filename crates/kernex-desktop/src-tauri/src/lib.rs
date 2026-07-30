use std::collections::{HashMap, VecDeque};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use agent_core::{
    AgentError, AgentEvent, AgentRunConfig, Approver, AuditedApprover, AuthManager, AuthStatus,
    CodexAccountStatus, CodexRateLimits, CommandOutput, CommandRunner, CommandSpec, EventSink,
    FileEditor, FileRecord, GitRepository, InstructionSet, KernexConfig, KernexSettings,
    ModelProvider, OAuthConfig, PermissionDecision, PermissionGate, PermissionMode,
    PermissionPolicy, PermissionRequest, ProjectGrantStore, ProjectIndex, ProviderConfig,
    ProviderKind, ProviderModel, RuntimeError, SecretValue, SessionRecord, SessionRecorder,
    SessionStatus, SessionStore, Workspace, codex_account_status, codex_login_chatgpt,
    codex_logout, codex_models, codex_rate_limits, prepare_http_provider, run_agent_turn,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Clone)]
struct ActiveRun {
    session_id: String,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
struct DesktopState {
    active: Arc<Mutex<Option<ActiveRun>>>,
    approvals: Arc<ApprovalBroker>,
}

#[derive(Clone, Serialize)]
struct PendingApproval {
    id: u64,
    request: PermissionRequest,
}

#[derive(Default)]
struct ApprovalBroker {
    next_id: AtomicU64,
    pending: Mutex<VecDeque<PendingApproval>>,
    responses: Mutex<HashMap<u64, PermissionDecision>>,
    response_ready: Condvar,
}

impl ApprovalBroker {
    fn request(&self, app: &AppHandle, request: &PermissionRequest) -> PermissionDecision {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let pending = PendingApproval {
            id,
            request: request.clone(),
        };
        if let Ok(mut queue) = self.pending.lock() {
            queue.push_back(pending.clone());
        } else {
            return PermissionDecision::Deny;
        }
        if app.emit("permission-request", pending).is_err() {
            return PermissionDecision::Deny;
        }
        let Ok(mut responses) = self.responses.lock() else {
            return PermissionDecision::Deny;
        };
        loop {
            if let Some(decision) = responses.remove(&id) {
                return decision;
            }
            let Ok(guard) = self.response_ready.wait(responses) else {
                return PermissionDecision::Deny;
            };
            responses = guard;
        }
    }

    fn respond(&self, id: u64, decision: PermissionDecision) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.retain(|item| item.id != id);
        }
        if let Ok(mut responses) = self.responses.lock() {
            responses.insert(id, decision);
            self.response_ready.notify_all();
        }
    }

    fn deny_all(&self) {
        let ids = self
            .pending
            .lock()
            .map(|mut pending| pending.drain(..).map(|item| item.id).collect::<Vec<_>>())
            .unwrap_or_default();
        if let Ok(mut responses) = self.responses.lock() {
            for id in ids {
                responses.insert(id, PermissionDecision::Deny);
            }
            self.response_ready.notify_all();
        }
    }
}

struct DesktopApprover {
    app: AppHandle,
    broker: Arc<ApprovalBroker>,
}

impl Approver for DesktopApprover {
    fn decide(&self, request: &PermissionRequest) -> PermissionDecision {
        self.broker.request(&self.app, request)
    }
}

struct DesktopEvents(AppHandle);

impl EventSink for DesktopEvents {
    fn emit(&self, event: AgentEvent) {
        let _ = self.0.emit("agent-event", event);
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartAgentRequest {
    workspace: String,
    task: String,
    provider: ProviderKind,
    model: String,
    base_url: Option<String>,
    auth_profile: Option<String>,
    permission_mode: PermissionMode,
    max_steps: usize,
    session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentFinished {
    session_id: String,
    error: Option<String>,
    cancelled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceOverview {
    path: String,
    is_git_repository: bool,
    files: Vec<FileRecord>,
    instructions: Vec<String>,
    git_status: String,
    mcp_servers: Vec<String>,
    language_servers: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ProviderSummary {
    kind: ProviderKind,
    base_url: String,
    api_key_environment: Option<String>,
    oauth_pkce: bool,
    managed_oauth: bool,
}

#[tauri::command]
async fn start_agent(
    app: AppHandle,
    state: State<'_, DesktopState>,
    request: StartAgentRequest,
) -> Result<String, String> {
    if state
        .active
        .lock()
        .map_err(|_| "desktop run state is unavailable".to_owned())?
        .is_some()
    {
        return Err("an agent operation is already running".into());
    }
    let workspace = Arc::new(Workspace::open(&request.workspace).map_err(error_string)?);
    if request.task.trim().is_empty() {
        return Err("task cannot be empty".into());
    }
    if request.model.trim().is_empty() {
        return Err("model cannot be empty".into());
    }
    let store = Arc::new(SessionStore::open_default().map_err(error_string)?);
    let mut session = match request.session_id {
        Some(ref id) => store
            .get(id)
            .map_err(error_string)?
            .ok_or_else(|| format!("session `{id}` does not exist"))?,
        None => SessionRecord::new(
            workspace.root().display().to_string(),
            request.provider.to_string(),
            request.model.clone(),
        ),
    };
    session.provider = request.provider.to_string();
    session.model.clone_from(&request.model);
    session.status = SessionStatus::Active;
    let provider_thread_id = session.provider_thread_id.clone();
    let history = session.messages.clone();
    let session_id = session.id.clone();
    let recorder = Arc::new(
        SessionRecorder::new(store, session, Arc::new(DesktopEvents(app.clone())))
            .map_err(error_string)?,
    );
    let approver: Arc<dyn Approver> = Arc::new(AuditedApprover::new(
        Arc::new(DesktopApprover {
            app: app.clone(),
            broker: state.approvals.clone(),
        }),
        recorder.clone(),
    ));
    let cancelled = Arc::new(AtomicBool::new(false));
    *state
        .active
        .lock()
        .map_err(|_| "desktop run state is unavailable".to_owned())? = Some(ActiveRun {
        session_id: session_id.clone(),
        cancelled: cancelled.clone(),
    });
    let active = state.active.clone();
    let mut provider = ProviderConfig::for_kind(request.provider, request.model);
    if let Some(base_url) = request.base_url {
        provider.base_url = base_url;
    }
    provider.auth_profile = request.auth_profile;
    let finished_session = session_id.clone();
    tauri::async_runtime::spawn(async move {
        let result = run_agent_turn(
            AgentRunConfig {
                workspace: workspace.root().to_path_buf(),
                provider,
                permission_mode: request.permission_mode,
                max_steps: request.max_steps,
                provider_thread_id,
            },
            request.task,
            history,
            approver,
            recorder.clone(),
            cancelled,
        )
        .await;
        let (error, was_cancelled) = match result {
            Ok(result) => {
                let error = recorder
                    .complete(&result)
                    .err()
                    .map(|error| error.to_string());
                (error, false)
            }
            Err(error) => {
                let was_cancelled = matches!(&error, RuntimeError::Agent(AgentError::Cancelled));
                let _ = recorder.fail(was_cancelled);
                (Some(error.to_string()), was_cancelled)
            }
        };
        if let Ok(mut current) = active.lock()
            && current
                .as_ref()
                .is_some_and(|run| run.session_id == finished_session)
        {
            *current = None;
        }
        let _ = app.emit(
            "agent-finished",
            AgentFinished {
                session_id: finished_session,
                error,
                cancelled: was_cancelled,
            },
        );
    });
    Ok(session_id)
}

#[tauri::command]
fn cancel_agent(state: State<'_, DesktopState>) -> Result<(), String> {
    let active = state
        .active
        .lock()
        .map_err(|_| "desktop run state is unavailable".to_owned())?;
    let Some(active) = active.as_ref() else {
        return Err("no agent operation is running".into());
    };
    active.cancelled.store(true, Ordering::Relaxed);
    state.approvals.deny_all();
    Ok(())
}

#[tauri::command]
fn respond_permission(
    state: State<'_, DesktopState>,
    id: u64,
    decision: String,
) -> Result<(), String> {
    let decision = match decision.as_str() {
        "allow_once" => PermissionDecision::AllowOnce,
        "allow_for_session" => PermissionDecision::AllowForSession,
        "allow_for_project" => PermissionDecision::AllowForProject,
        "deny" => PermissionDecision::Deny,
        _ => return Err(format!("unknown permission decision `{decision}`")),
    };
    state.approvals.respond(id, decision);
    Ok(())
}

#[tauri::command]
fn workspace_overview(path: String) -> Result<WorkspaceOverview, String> {
    let workspace = Arc::new(Workspace::open(&path).map_err(error_string)?);
    let index = ProjectIndex::build(&workspace).map_err(error_string)?;
    let permissions = Arc::new(PermissionGate::new(PermissionPolicy::default(), None));
    let git = GitRepository::new(workspace.clone(), permissions);
    let is_git_repository = workspace.root().join(".git").exists();
    let git_status = if is_git_repository {
        git.status().map(|output| output.stdout).unwrap_or_default()
    } else {
        String::new()
    };
    let instructions = InstructionSet::discover(&workspace)
        .map_err(error_string)?
        .documents()
        .iter()
        .map(|document| document.path.clone())
        .collect();
    let config = KernexConfig::load(&workspace).map_err(error_string)?;
    Ok(WorkspaceOverview {
        path: workspace.root().display().to_string(),
        is_git_repository,
        files: index.files().to_vec(),
        instructions,
        git_status,
        mcp_servers: config
            .mcp_servers
            .iter()
            .map(|server| server.name.clone())
            .collect(),
        language_servers: config
            .language_servers
            .iter()
            .map(|server| server.server.language_id.clone())
            .collect(),
    })
}

#[tauri::command]
fn read_project_file(workspace: String, path: String) -> Result<String, String> {
    let workspace = Workspace::open(workspace).map_err(error_string)?;
    let resolved = workspace.resolve_existing(path).map_err(error_string)?;
    if workspace.is_sensitive_path(&resolved) {
        return Err("sensitive files can only be read through an approved agent tool call".into());
    }
    workspace.read_text(resolved).map_err(error_string)
}

#[tauri::command]
fn git_status(workspace: String) -> Result<String, String> {
    let workspace = Arc::new(Workspace::open(workspace).map_err(error_string)?);
    let permissions = Arc::new(PermissionGate::new(PermissionPolicy::default(), None));
    GitRepository::new(workspace, permissions)
        .status()
        .map(|output| output.stdout)
        .map_err(error_string)
}

#[tauri::command]
fn git_diff(workspace: String, staged: bool) -> Result<String, String> {
    let workspace = Arc::new(Workspace::open(workspace).map_err(error_string)?);
    let permissions = Arc::new(PermissionGate::new(PermissionPolicy::default(), None));
    GitRepository::new(workspace, permissions)
        .diff(staged)
        .map(|output| output.stdout)
        .map_err(error_string)
}

#[tauri::command]
fn git_log(workspace: String, limit: usize) -> Result<String, String> {
    let workspace = Arc::new(Workspace::open(workspace).map_err(error_string)?);
    let permissions = Arc::new(PermissionGate::new(PermissionPolicy::default(), None));
    GitRepository::new(workspace, permissions)
        .log(limit)
        .map(|output| output.stdout)
        .map_err(error_string)
}

#[tauri::command]
async fn run_terminal(
    app: AppHandle,
    state: State<'_, DesktopState>,
    workspace: String,
    command: Vec<String>,
) -> Result<CommandOutput, String> {
    let Some((program, args)) = command.split_first() else {
        return Err("a command is required".into());
    };
    let workspace = Arc::new(Workspace::open(workspace).map_err(error_string)?);
    let grants = Arc::new(ProjectGrantStore::open_default().map_err(error_string)?);
    let approver = Arc::new(DesktopApprover {
        app,
        broker: state.approvals.clone(),
    });
    let permissions = Arc::new(PermissionGate::for_project(
        PermissionPolicy::for_mode(PermissionMode::Ask),
        Some(approver),
        workspace.root().display().to_string(),
        grants,
    ));
    CommandRunner::new(workspace, permissions)
        .run(&CommandSpec::new(program, args))
        .await
        .map_err(error_string)
}

#[tauri::command]
fn list_sessions(workspace: Option<String>, limit: usize) -> Result<Vec<SessionRecord>, String> {
    let store = SessionStore::open_default().map_err(error_string)?;
    match workspace {
        Some(path) => {
            let workspace = Workspace::open(path).map_err(error_string)?;
            store
                .list_for_workspace(&workspace.root().display().to_string(), limit)
                .map_err(error_string)
        }
        None => store.list(limit).map_err(error_string),
    }
}

#[tauri::command]
fn load_session(id: String) -> Result<SessionRecord, String> {
    SessionStore::open_default()
        .map_err(error_string)?
        .get(&id)
        .map_err(error_string)?
        .ok_or_else(|| format!("session `{id}` does not exist"))
}

#[tauri::command]
fn delete_session(id: String) -> Result<bool, String> {
    SessionStore::open_default()
        .map_err(error_string)?
        .delete(&id)
        .map_err(error_string)
}

#[tauri::command]
fn get_settings() -> Result<KernexSettings, String> {
    KernexSettings::load_default().map_err(error_string)
}

#[tauri::command]
fn save_settings(settings: KernexSettings) -> Result<(), String> {
    settings.save_default().map_err(error_string)
}

#[tauri::command]
async fn discover_models(
    app: AppHandle,
    state: State<'_, DesktopState>,
    provider: String,
    model: String,
    base_url: Option<String>,
    auth_profile: Option<String>,
) -> Result<Vec<ProviderModel>, String> {
    let provider = ProviderKind::from_str(&provider).map_err(error_string)?;
    if provider == ProviderKind::Codex {
        return codex_models().await.map_err(error_string);
    }
    let mut config = ProviderConfig::for_kind(
        provider,
        if model.trim().is_empty() {
            "model-catalog".to_owned()
        } else {
            model
        },
    );
    if let Some(base_url) = base_url.filter(|value| !value.trim().is_empty()) {
        config.base_url = base_url;
    }
    config.auth_profile = auth_profile;
    let approver = Arc::new(DesktopApprover {
        app,
        broker: state.approvals.clone(),
    });
    let permissions = Arc::new(PermissionGate::new(
        PermissionPolicy::for_mode(PermissionMode::Ask),
        Some(approver),
    ));
    prepare_http_provider(config, permissions)
        .await
        .map_err(error_string)?
        .models()
        .await
        .map_err(error_string)
}

#[tauri::command]
fn project_config(workspace: String) -> Result<String, String> {
    let workspace = Workspace::open(workspace).map_err(error_string)?;
    let path = workspace.root().join(".kernex/config.toml");
    if path.exists() {
        workspace.read_text(path).map_err(error_string)
    } else {
        Ok("# Project-specific provider, MCP, and language-server settings.\n".into())
    }
}

#[tauri::command]
fn save_project_config(
    app: AppHandle,
    state: State<'_, DesktopState>,
    workspace: String,
    contents: String,
) -> Result<String, String> {
    KernexConfig::parse(&contents).map_err(error_string)?;
    let workspace = Arc::new(Workspace::open(workspace).map_err(error_string)?);
    let grants = Arc::new(ProjectGrantStore::open_default().map_err(error_string)?);
    let approver = Arc::new(DesktopApprover {
        app,
        broker: state.approvals.clone(),
    });
    let permissions = Arc::new(PermissionGate::for_project(
        PermissionPolicy::for_mode(PermissionMode::Ask),
        Some(approver),
        workspace.root().display().to_string(),
        grants,
    ));
    FileEditor::new(workspace, permissions)
        .write_text(".kernex/config.toml", &contents)
        .map(|change| change.diff)
        .map_err(error_string)
}

#[tauri::command]
fn providers() -> Vec<ProviderSummary> {
    ProviderKind::ALL
        .into_iter()
        .map(|kind| {
            let config = ProviderConfig::for_kind(kind, "");
            ProviderSummary {
                kind,
                base_url: config.base_url,
                api_key_environment: config.api_key_env,
                oauth_pkce: kind == ProviderKind::Gemini,
                managed_oauth: kind == ProviderKind::Codex,
            }
        })
        .collect()
}

#[tauri::command]
fn auth_status() -> Result<Vec<AuthStatus>, String> {
    AuthManager::open_default()
        .map_err(error_string)?
        .statuses()
        .map_err(error_string)
}

#[tauri::command]
async fn codex_account() -> Result<CodexAccountStatus, String> {
    codex_account_status(false).await.map_err(error_string)
}

#[tauri::command]
async fn codex_login() -> Result<CodexAccountStatus, String> {
    codex_login_chatgpt().await.map_err(error_string)
}

#[tauri::command]
async fn codex_sign_out() -> Result<(), String> {
    codex_logout().await.map_err(error_string)
}

#[tauri::command]
async fn codex_limits() -> Result<CodexRateLimits, String> {
    codex_rate_limits().await.map_err(error_string)
}

#[tauri::command]
fn auth_login_api_key(profile: String, provider: String, api_key: String) -> Result<(), String> {
    let provider = ProviderKind::from_str(&provider).map_err(error_string)?;
    if provider == ProviderKind::Codex {
        return Err("Codex subscription access uses managed ChatGPT OAuth".into());
    }
    AuthManager::open_default()
        .map_err(error_string)?
        .login_api_key(profile, provider, SecretValue::new(api_key))
        .map(|_| ())
        .map_err(error_string)
}

#[tauri::command]
fn auth_login_environment(
    profile: String,
    provider: String,
    variable: String,
) -> Result<(), String> {
    let provider = ProviderKind::from_str(&provider).map_err(error_string)?;
    if provider == ProviderKind::Codex {
        return Err("Codex subscription access uses managed ChatGPT OAuth".into());
    }
    AuthManager::open_default()
        .map_err(error_string)?
        .login_environment(profile, provider, variable)
        .map(|_| ())
        .map_err(error_string)
}

#[tauri::command]
async fn auth_login_oauth(
    profile: String,
    provider: String,
    client_id: String,
    authorization_url: Option<String>,
    token_url: Option<String>,
    scopes: Vec<String>,
    resource_project: Option<String>,
) -> Result<(), String> {
    let provider = ProviderKind::from_str(&provider).map_err(error_string)?;
    let config =
        if provider == ProviderKind::Gemini && authorization_url.is_none() && token_url.is_none() {
            let resource_project = resource_project
                .filter(|project| !project.trim().is_empty())
                .ok_or_else(|| "Google OAuth requires a Google Cloud project ID".to_owned())?;
            OAuthConfig::google(client_id, resource_project)
        } else if provider == ProviderKind::Custom {
            OAuthConfig {
                authorization_url: authorization_url.ok_or_else(|| {
                    "custom OAuth requires an official authorization URL".to_owned()
                })?,
                token_url: token_url
                    .ok_or_else(|| "custom OAuth requires an official token URL".to_owned())?,
                client_id,
                scopes,
                extra_authorization_parameters: Default::default(),
                resource_project: None,
            }
        } else {
            return Err(format!(
                "{provider} has no official third-party OAuth flow configured by Kernex"
            ));
        };
    AuthManager::open_default()
        .map_err(error_string)?
        .login_oauth(profile, provider, config)
        .await
        .map(|_| ())
        .map_err(error_string)
}

#[tauri::command]
fn auth_logout(profile: String) -> Result<(), String> {
    AuthManager::open_default()
        .map_err(error_string)?
        .logout(&profile)
        .map_err(error_string)
}

#[tauri::command]
fn auth_use(profile: String) -> Result<(), String> {
    let manager = AuthManager::open_default().map_err(error_string)?;
    let selected = manager
        .profiles()
        .map_err(error_string)?
        .into_iter()
        .find(|candidate| candidate.name == profile)
        .ok_or_else(|| format!("profile `{profile}` does not exist"))?;
    manager
        .set_active(selected.provider, &profile)
        .map_err(error_string)
}

fn error_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(DesktopState::default())
        .invoke_handler(tauri::generate_handler![
            start_agent,
            cancel_agent,
            respond_permission,
            workspace_overview,
            read_project_file,
            git_status,
            git_diff,
            git_log,
            run_terminal,
            list_sessions,
            load_session,
            delete_session,
            get_settings,
            save_settings,
            discover_models,
            project_config,
            save_project_config,
            providers,
            auth_status,
            codex_account,
            codex_login,
            codex_sign_out,
            codex_limits,
            auth_login_api_key,
            auth_login_environment,
            auth_login_oauth,
            auth_logout,
            auth_use,
        ])
        .setup(|app| {
            if let Ok(settings) = KernexSettings::load_default()
                && settings.theme == "dark"
                && let Some(window) = app.get_webview_window("main")
            {
                let _ = window.set_theme(Some(tauri::Theme::Dark));
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the Kernex desktop application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_provider_advertises_managed_oauth_to_the_desktop() {
        let codex = providers()
            .into_iter()
            .find(|provider| provider.kind == ProviderKind::Codex)
            .expect("Codex provider summary");
        assert!(codex.managed_oauth);
        assert!(!codex.oauth_pkce);
        assert!(codex.base_url.is_empty());
        assert!(codex.api_key_environment.is_none());
    }
}
