use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use thiserror::Error;

use crate::agent::{AgentEngine, AgentError, AgentRunResult, EventSink, Toolbox};
use crate::auth::{AuthError, AuthManager, AuthMethod};
use crate::codex_app_server::{CodexAppServerError, CodexTurnConfig, run_codex_turn};
use crate::config::{ConfigError, KernexConfig};
use crate::permission::{
    Approver, PermissionDecision, PermissionGate, PermissionMode, PermissionPolicy,
    PermissionRequest, ProjectGrantStore,
};
use crate::provider::{
    HttpModelProvider, Message, ProviderConfig, ProviderCredentialKind, ProviderError, ProviderKind,
};
use crate::workspace::{Workspace, WorkspaceError};

#[derive(Debug, Clone)]
pub struct AgentRunConfig {
    pub workspace: PathBuf,
    pub provider: ProviderConfig,
    pub permission_mode: PermissionMode,
    pub max_steps: usize,
    /// Provider-owned conversation identifier used when resuming delegated runtimes.
    pub provider_thread_id: Option<String>,
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Permission(#[from] crate::permission::PermissionError),
    #[error(transparent)]
    Authentication(#[from] AuthError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Configuration(#[from] ConfigError),
    #[error(transparent)]
    Tool(#[from] crate::agent::ToolError),
    #[error(transparent)]
    Agent(#[from] AgentError),
    #[error(transparent)]
    CodexAppServer(#[from] CodexAppServerError),
}

/// Resolves a configured authentication profile and builds a permissioned HTTP provider.
pub async fn prepare_http_provider(
    config: ProviderConfig,
    permissions: Arc<PermissionGate>,
) -> Result<HttpModelProvider, RuntimeError> {
    let mut provider = HttpModelProvider::new(config.clone(), permissions)?;
    if let Some(profile_name) = &config.auth_profile {
        let authentication = AuthManager::open_default()?;
        let profile = authentication
            .profiles()?
            .into_iter()
            .find(|profile| &profile.name == profile_name)
            .ok_or_else(|| AuthError::MissingProfile(profile_name.clone()))?;
        let credential = authentication.resolve(profile_name).await?;
        let kind = if profile.method == AuthMethod::OAuthPkce {
            ProviderCredentialKind::OAuthBearer
        } else {
            ProviderCredentialKind::ApiKey
        };
        provider = provider.with_credential(credential, kind);
        if config.kind == crate::provider::ProviderKind::Gemini
            && let Some(project) = profile.oauth_resource_project
        {
            provider = provider.with_google_resource_project(project);
        }
    }
    Ok(provider)
}

/// Runs one agent turn through the same assembly path for every presentation layer.
pub async fn run_agent_turn(
    config: AgentRunConfig,
    task: impl Into<String>,
    history: Vec<Message>,
    approver: Arc<dyn Approver>,
    events: Arc<dyn EventSink>,
    cancelled: Arc<AtomicBool>,
) -> Result<AgentRunResult, RuntimeError> {
    let workspace = Arc::new(Workspace::open(&config.workspace)?);
    let project_scope = workspace.root().display().to_string();
    let project_grants = Arc::new(ProjectGrantStore::open_default()?);
    let permissions = Arc::new(PermissionGate::for_project(
        PermissionPolicy::for_mode(config.permission_mode),
        Some(approver),
        project_scope,
        project_grants,
    ));
    if config.provider.kind == ProviderKind::Codex {
        let result = run_codex_turn(
            CodexTurnConfig {
                workspace: workspace.root().to_path_buf(),
                model: config.provider.model,
                permission_mode: config.permission_mode,
                provider_thread_id: config.provider_thread_id,
            },
            task,
            history,
            Arc::new(GateApprover(permissions)),
            events,
            cancelled,
        )
        .await;
        return match result {
            Ok(result) => Ok(result),
            Err(CodexAppServerError::Cancelled) => Err(AgentError::Cancelled.into()),
            Err(CodexAppServerError::EmptyTask) => Err(AgentError::EmptyTask.into()),
            Err(error) => Err(error.into()),
        };
    }
    let provider = prepare_http_provider(config.provider.clone(), permissions.clone()).await?;

    let extension_config = KernexConfig::load(&workspace)?;
    let toolbox = Toolbox::new(workspace, permissions)?
        .connect_mcp(extension_config.mcp_servers)
        .await?
        .connect_language_servers(extension_config.language_servers)
        .await?;
    AgentEngine::new(Arc::new(provider), toolbox, events)
        .with_max_steps(config.max_steps)
        .with_cancellation(cancelled)
        .run_with_history(task, history)
        .await
        .map_err(RuntimeError::from)
}

/// Applies Kernex's session/project grant semantics before answering an App Server approval.
struct GateApprover(Arc<PermissionGate>);

impl Approver for GateApprover {
    fn decide(&self, request: &PermissionRequest) -> PermissionDecision {
        if self.0.authorize(request).is_ok() {
            PermissionDecision::AllowOnce
        } else {
            PermissionDecision::Deny
        }
    }
}
