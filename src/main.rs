use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use agent_core::{
    AgentError, AgentEvent, AgentRunConfig, Approver, AuditedApprover, AuthManager, CommandRunner,
    CommandSpec, CompletionRequest, EventSink, GitRepository, KernexConfig, KernexSettings,
    Message, ModelProvider, OAuthConfig, PermissionDecision, PermissionGate, PermissionMode,
    PermissionPolicy, PermissionRequest, PluginRegistry, ProjectIndex, ProviderConfig,
    ProviderKind, ProviderStreamEvent, Role, RuntimeError, SecretValue, SessionRecord,
    SessionRecorder, SessionStatus, SessionStore, SyntaxAnalyzer, VERSION, Workspace,
    prepare_http_provider, run_agent_turn,
};
use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use dialoguer::{Confirm, Input, Password, Select};

#[derive(Debug, Parser)]
#[command(
    name = "kernex",
    version = VERSION,
    about = "A safe, provider-independent AI coding agent",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    /// Project directory Kernex is allowed to access.
    #[arg(short = 'C', long, default_value = ".", global = true)]
    project: PathBuf,

    /// Approve protected actions for this invocation without prompting.
    #[arg(long, global = true)]
    yes: bool,

    /// Permission policy for agent tools.
    #[arg(long, value_enum, global = true)]
    permission_mode: Option<PermissionModeArg>,

    /// Run one task directly, for example `kernex "Fix the failing tests"`.
    #[arg(value_name = "TASK")]
    task: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum PermissionModeArg {
    ReadOnly,
    Ask,
    AutoSafe,
    FullAccess,
}

impl From<PermissionModeArg> for PermissionMode {
    fn from(value: PermissionModeArg) -> Self {
        match value {
            PermissionModeArg::ReadOnly => Self::ReadOnly,
            PermissionModeArg::Ask => Self::Ask,
            PermissionModeArg::AutoSafe => Self::AutoSafe,
            PermissionModeArg::FullAccess => Self::FullAccess,
        }
    }
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Summarize files and detected languages in a project.
    Inspect {
        #[arg(long)]
        json: bool,
    },
    /// Search text files while respecting Git ignore rules.
    Search {
        query: String,
        #[arg(long)]
        case_sensitive: bool,
        #[arg(long, default_value_t = 50)]
        max_results: usize,
        #[arg(long)]
        json: bool,
    },
    /// Print a tree-sitter structural outline for a source file.
    Outline { path: PathBuf },
    /// Show concise Git working-tree status.
    Status,
    /// Show the reviewable Git diff.
    Diff {
        #[arg(long)]
        staged: bool,
    },
    /// Review the current Git status and working-tree diff.
    Review,
    /// Execute one argument-vector command after permission review.
    Exec(ExecArgs),
    /// Send a single prompt to a configured model provider.
    Ask(AskArgs),
    /// Run an agent task with project tools and transparent permission prompts.
    Run(RunArgs),
    /// Resume a stored session, or the most recent session for this project.
    Resume {
        session: Option<String>,
        #[arg(long)]
        task: Option<String>,
    },
    /// Create a starter KERNEX.md instruction file.
    Init {
        #[arg(long)]
        force: bool,
    },
    /// Manage secure authentication profiles.
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
    },
    /// List known providers and the currently selected model.
    Models {
        #[arg(long)]
        json: bool,
        /// Contact the selected provider and list its live model catalog.
        #[arg(long)]
        discover: bool,
    },
    /// List supported provider adapters and their safe defaults.
    Providers {
        #[arg(long)]
        json: bool,
    },
    /// List locally stored sessions.
    Sessions {
        #[arg(long, default_value_t = 30)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Show discovered plugins and configured MCP/LSP servers without starting them.
    Extensions,
}

#[derive(Debug, Subcommand)]
enum AuthCommands {
    /// Sign in with an API key, environment variable, or official OAuth PKCE flow.
    Login(AuthLoginArgs),
    /// Delete a profile and its credentials from native secure storage.
    Logout { profile: Option<String> },
    /// Show authentication profiles without revealing credentials.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Select an existing profile for its provider.
    Use { profile: String },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum AuthMethodArg {
    ApiKey,
    Environment,
    OAuth,
}

#[derive(Debug, Args)]
struct AuthLoginArgs {
    #[arg(long)]
    provider: Option<String>,
    #[arg(long, value_enum)]
    method: Option<AuthMethodArg>,
    #[arg(long, default_value = "personal")]
    profile: String,
    #[arg(long)]
    environment_variable: Option<String>,
    #[arg(long)]
    client_id: Option<String>,
    /// Google Cloud project used for Gemini OAuth quota and API access.
    #[arg(long)]
    google_project: Option<String>,
    #[arg(long)]
    authorization_url: Option<String>,
    #[arg(long)]
    token_url: Option<String>,
    #[arg(long = "scope")]
    scopes: Vec<String>,
}

#[derive(Debug, Args)]
struct ExecArgs {
    #[arg(long, default_value_t = 120)]
    timeout: u64,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<String>,
}

#[derive(Debug, Args)]
struct AskArgs {
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    base_url: Option<String>,
    #[arg(long)]
    api_key_env: Option<String>,
    #[arg(long = "header-env", value_name = "HEADER=ENV")]
    header_env: Vec<String>,
    prompt: String,
}

#[derive(Debug, Args)]
struct RunArgs {
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    base_url: Option<String>,
    #[arg(long)]
    api_key_env: Option<String>,
    #[arg(long = "header-env", value_name = "HEADER=ENV")]
    header_env: Vec<String>,
    #[arg(long, default_value_t = 24)]
    max_steps: usize,
    task: String,
}

struct TerminalApprover {
    assume_yes: bool,
}

impl Approver for TerminalApprover {
    fn decide(&self, request: &PermissionRequest) -> PermissionDecision {
        eprintln!(
            "\nPermission request: {} [{:?}]\nResource: {}",
            request.summary, request.risk, request.resource
        );
        for detail in &request.details {
            eprintln!("{detail}");
        }
        if self.assume_yes {
            eprintln!("Approved for this session by --yes.");
            return PermissionDecision::AllowForSession;
        }
        if !io::stdin().is_terminal() {
            eprintln!(
                "Denied because no interactive terminal is available; pass --yes explicitly."
            );
            return PermissionDecision::Deny;
        }
        eprint!("Allow? [y]es / [s]ession / [p]roject / [N]o: ");
        if io::stderr().flush().is_err() {
            return PermissionDecision::Deny;
        }
        let mut answer = String::new();
        if io::stdin().read_line(&mut answer).is_err() {
            return PermissionDecision::Deny;
        }
        match answer.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => PermissionDecision::AllowOnce,
            "s" | "session" => PermissionDecision::AllowForSession,
            "p" | "project" => PermissionDecision::AllowForProject,
            _ => PermissionDecision::Deny,
        }
    }
}

struct TerminalEvents;

impl EventSink for TerminalEvents {
    fn emit(&self, event: AgentEvent) {
        match event {
            AgentEvent::Started {
                provider, model, ..
            } => {
                eprintln!("[kernex] agent started with {provider}/{model}")
            }
            AgentEvent::ModelRequested { step } => {
                eprintln!("[kernex] step {step}: requesting model response")
            }
            AgentEvent::ModelDelta { event, .. } => match event {
                ProviderStreamEvent::TextDelta { text } => {
                    print!("{text}");
                    let _ = io::stdout().flush();
                }
                ProviderStreamEvent::ToolCallDelta { name, .. } => {
                    if let Some(name) = name {
                        eprintln!("\n[kernex] streaming tool call {name}");
                    }
                }
                ProviderStreamEvent::Usage { usage } => {
                    if usage.input_tokens.is_some() || usage.output_tokens.is_some() {
                        eprintln!(
                            "\n[kernex] tokens: {} input, {} output",
                            usage.input_tokens.unwrap_or(0),
                            usage.output_tokens.unwrap_or(0)
                        );
                    }
                }
            },
            AgentEvent::ModelResponded { tool_calls, .. } => {
                if !tool_calls.is_empty() {
                    eprintln!(
                        "\n[kernex] model requested {} tool action(s)",
                        tool_calls.len()
                    );
                } else {
                    println!();
                }
            }
            AgentEvent::ToolStarted { call, .. } => {
                eprintln!("[kernex] tool {} {}", call.name, call.arguments)
            }
            AgentEvent::ToolFinished {
                name, result, diff, ..
            } => {
                if let Some(diff) = diff {
                    eprintln!("[kernex] {name} applied:\n{diff}");
                } else {
                    let preview: String = result.chars().take(500).collect();
                    eprintln!("[kernex] {name} completed: {preview}");
                    if result.chars().count() > 500 {
                        eprintln!("[kernex] result preview truncated");
                    }
                }
            }
            AgentEvent::ToolFailed { name, error, .. } => {
                eprintln!("[kernex] {name} failed: {error}")
            }
            AgentEvent::Completed { steps } => {
                eprintln!("[kernex] completed after {steps} step(s)")
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut settings = KernexSettings::load_default().unwrap_or_default();
    if let Some(mode) = cli.permission_mode {
        settings.permission_mode = mode.into();
    }

    if let Some(task) = cli.task {
        let workspace = open_workspace(&cli.project)?;
        let provider =
            selected_provider(&workspace, &settings, None, None, None, None, Vec::new())?;
        run_one_task(
            workspace,
            provider,
            settings.permission_mode,
            task,
            24,
            cli.yes,
            None,
        )
        .await?;
        return Ok(());
    }

    let Some(command) = cli.command else {
        return interactive(cli.project, settings, cli.yes, None).await;
    };
    match command {
        Commands::Auth { command } => handle_auth(command, &mut settings).await?,
        Commands::Models { json, discover } => {
            list_models_and_providers(&settings, json, discover, cli.yes).await?
        }
        Commands::Providers { json } => {
            list_models_and_providers(&settings, json, false, false).await?
        }
        other => {
            let workspace = open_workspace(&cli.project)?;
            let permissions = basic_permissions(cli.yes);
            match other {
                Commands::Inspect { json } => inspect(&workspace, json)?,
                Commands::Search {
                    query,
                    case_sensitive,
                    max_results,
                    json,
                } => search(&workspace, &query, case_sensitive, max_results, json)?,
                Commands::Outline { path } => {
                    let outline = SyntaxAnalyzer::outline(&workspace, path)?;
                    println!("{}", serde_json::to_string_pretty(&outline)?);
                }
                Commands::Status => {
                    print!(
                        "{}",
                        GitRepository::new(workspace, permissions).status()?.stdout
                    );
                }
                Commands::Diff { staged } => {
                    print!(
                        "{}",
                        GitRepository::new(workspace, permissions)
                            .diff(staged)?
                            .stdout
                    );
                }
                Commands::Review => review(workspace, permissions)?,
                Commands::Exec(args) => run_command(workspace, permissions, args).await?,
                Commands::Ask(args) => {
                    let provider = selected_provider(
                        &workspace,
                        &settings,
                        args.provider,
                        args.model,
                        args.base_url,
                        args.api_key_env,
                        args.header_env,
                    )?;
                    ask_model(permissions, provider, args.prompt).await?;
                }
                Commands::Run(args) => {
                    let provider = selected_provider(
                        &workspace,
                        &settings,
                        args.provider,
                        args.model,
                        args.base_url,
                        args.api_key_env,
                        args.header_env,
                    )?;
                    run_one_task(
                        workspace,
                        provider,
                        settings.permission_mode,
                        args.task,
                        args.max_steps,
                        cli.yes,
                        None,
                    )
                    .await?;
                }
                Commands::Resume { session, task } => {
                    let stored = load_session(&workspace, session.as_deref())?;
                    if let Some(task) = task {
                        let mut provider = selected_provider(
                            &workspace,
                            &settings,
                            Some(stored.provider.clone()),
                            Some(stored.model.clone()),
                            None,
                            None,
                            Vec::new(),
                        )?;
                        provider
                            .auth_profile
                            .clone_from(&settings.provider.auth_profile);
                        run_one_task(
                            workspace,
                            provider,
                            settings.permission_mode,
                            task,
                            24,
                            cli.yes,
                            Some(stored),
                        )
                        .await?;
                    } else {
                        interactive(cli.project, settings, cli.yes, Some(stored)).await?;
                    }
                }
                Commands::Init { force } => initialize_instructions(&workspace, force)?,
                Commands::Sessions { limit, json } => list_sessions(&workspace, limit, json)?,
                Commands::Extensions => list_extensions(&workspace)?,
                Commands::Auth { .. } | Commands::Models { .. } | Commands::Providers { .. } => {
                    unreachable!()
                }
            }
        }
    }
    Ok(())
}

fn open_workspace(path: &Path) -> Result<Arc<Workspace>> {
    Ok(Arc::new(Workspace::open(path).with_context(|| {
        format!("cannot open project {}", path.display())
    })?))
}

fn basic_permissions(assume_yes: bool) -> Arc<PermissionGate> {
    Arc::new(PermissionGate::new(
        PermissionPolicy::default(),
        Some(Arc::new(TerminalApprover { assume_yes })),
    ))
}

async fn interactive(
    project: PathBuf,
    mut settings: KernexSettings,
    assume_yes: bool,
    resumed: Option<SessionRecord>,
) -> Result<()> {
    if !io::stdin().is_terminal() {
        bail!("interactive mode requires a terminal; pass a task or use `kernex run`");
    }
    let workspace = open_workspace(&project)?;
    settings.remember_project(workspace.root().display().to_string());
    let _ = settings.save_default();
    let mut provider = if let Some(session) = &resumed {
        selected_provider(
            &workspace,
            &settings,
            Some(session.provider.clone()),
            Some(session.model.clone()),
            None,
            None,
            Vec::new(),
        )?
    } else {
        selected_provider(&workspace, &settings, None, None, None, None, Vec::new())?
    };
    if provider.model.trim().is_empty() {
        provider.model = Input::new().with_prompt("Model ID").interact_text()?;
        settings.provider.name = provider.kind;
        settings.provider.model.clone_from(&provider.model);
        settings.save_default()?;
    }
    let mut session = resumed.unwrap_or_else(|| {
        SessionRecord::new(
            workspace.root().display().to_string(),
            provider.kind.to_string(),
            provider.model.clone(),
        )
    });
    print_markdown(&format!(
        "# Kernex\n\nWorkspace: `{}`  \nProvider: `{}/{}`\n\nType `/help` for commands or `Ctrl+C` to cancel a running operation.",
        workspace.root().display(),
        provider.kind,
        provider.model
    ));
    loop {
        let input = Input::<String>::new()
            .with_prompt("kernex")
            .allow_empty(true)
            .interact_text();
        let task = match input {
            Ok(task) => task.trim().to_owned(),
            Err(_) => break,
        };
        if task.is_empty() {
            continue;
        }
        if task.starts_with('/') {
            match handle_interactive_command(&task, &mut provider, &mut settings, &workspace)? {
                InteractiveAction::Continue => continue,
                InteractiveAction::Exit => break,
            }
        }
        let id = session.id.clone();
        run_one_task(
            workspace.clone(),
            provider.clone(),
            settings.permission_mode,
            task,
            24,
            assume_yes,
            Some(session),
        )
        .await?;
        session = SessionStore::open_default()?
            .get(&id)?
            .context("the active session disappeared from local storage")?;
    }
    Ok(())
}

enum InteractiveAction {
    Continue,
    Exit,
}

fn handle_interactive_command(
    input: &str,
    provider: &mut ProviderConfig,
    settings: &mut KernexSettings,
    workspace: &Workspace,
) -> Result<InteractiveAction> {
    let (command, value) = input.split_once(' ').unwrap_or((input, ""));
    match command {
        "/exit" | "/quit" => Ok(InteractiveAction::Exit),
        "/help" => {
            print_markdown(
                "## Interactive commands\n\n- `/model MODEL` select a model\n- `/provider PROVIDER` select a provider\n- `/sessions` list project sessions\n- `/help` show this help\n- `/exit` leave Kernex\n\nPress `Ctrl+C` while an agent is working to cancel it.",
            );
            Ok(InteractiveAction::Continue)
        }
        "/model" if !value.trim().is_empty() => {
            provider.model = value.trim().into();
            settings.provider.model.clone_from(&provider.model);
            settings.save_default()?;
            println!("Selected model {}.", provider.model);
            Ok(InteractiveAction::Continue)
        }
        "/provider" if !value.trim().is_empty() => {
            let kind = ProviderKind::from_str(value.trim())?;
            let model = provider.model.clone();
            *provider = ProviderConfig::for_kind(kind, model);
            settings.provider.name = kind;
            settings.save_default()?;
            println!("Selected provider {kind}.");
            Ok(InteractiveAction::Continue)
        }
        "/sessions" => {
            list_sessions(workspace, 20, false)?;
            Ok(InteractiveAction::Continue)
        }
        "/cancel" => {
            println!("Press Ctrl+C while an operation is running to cancel it.");
            Ok(InteractiveAction::Continue)
        }
        _ => {
            println!("Unknown interactive command. Use /help.");
            Ok(InteractiveAction::Continue)
        }
    }
}

async fn run_one_task(
    workspace: Arc<Workspace>,
    provider: ProviderConfig,
    permission_mode: PermissionMode,
    task: String,
    max_steps: usize,
    assume_yes: bool,
    existing: Option<SessionRecord>,
) -> Result<()> {
    if provider.model.trim().is_empty() {
        bail!("no model is configured; pass --model or set one in interactive mode");
    }
    let store = Arc::new(SessionStore::open_default()?);
    let mut session = existing.unwrap_or_else(|| {
        SessionRecord::new(
            workspace.root().display().to_string(),
            provider.kind.to_string(),
            provider.model.clone(),
        )
    });
    session.provider = provider.kind.to_string();
    session.model.clone_from(&provider.model);
    session.status = SessionStatus::Active;
    let history = session.messages.clone();
    let recorder = Arc::new(SessionRecorder::new(
        store,
        session,
        Arc::new(TerminalEvents),
    )?);
    let approver: Arc<dyn Approver> = Arc::new(AuditedApprover::new(
        Arc::new(TerminalApprover { assume_yes }),
        recorder.clone(),
    ));
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal_cancelled = cancelled.clone();
    let signal = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            signal_cancelled.store(true, Ordering::Relaxed);
        }
    });
    let result = run_agent_turn(
        AgentRunConfig {
            workspace: workspace.root().to_path_buf(),
            provider,
            permission_mode,
            max_steps,
        },
        task,
        history,
        approver,
        recorder.clone(),
        cancelled,
    )
    .await;
    signal.abort();
    match result {
        Ok(result) => {
            recorder.complete(&result)?;
            Ok(())
        }
        Err(error) => {
            let was_cancelled = matches!(error, RuntimeError::Agent(AgentError::Cancelled));
            recorder.fail(was_cancelled)?;
            Err(error.into())
        }
    }
}

fn selected_provider(
    workspace: &Workspace,
    settings: &KernexSettings,
    provider: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
    api_key_env: Option<String>,
    header_env: Vec<String>,
) -> Result<ProviderConfig> {
    let project = KernexConfig::load(workspace)?.provider;
    let selected_kind = provider
        .as_deref()
        .map(ProviderKind::from_str)
        .transpose()?
        .or_else(|| project.as_ref().map(|provider| provider.name))
        .unwrap_or(settings.provider.name);
    let selected_model = model
        .or_else(|| project.as_ref().map(|provider| provider.model.clone()))
        .filter(|model| !model.trim().is_empty())
        .unwrap_or_else(|| settings.provider.model.clone());
    let mut config = ProviderConfig::for_kind(selected_kind, selected_model);
    config.base_url = base_url
        .or_else(|| {
            project
                .as_ref()
                .and_then(|provider| provider.base_url.clone())
        })
        .or_else(|| settings.provider.base_url.clone())
        .unwrap_or(config.base_url);
    config.auth_profile = project
        .as_ref()
        .and_then(|provider| provider.auth_profile.clone())
        .or_else(|| settings.provider.auth_profile.clone());
    if let Some(variable) = api_key_env {
        config.api_key_env = Some(variable);
        config.auth_profile = None;
    }
    for mapping in header_env {
        let (header, variable) = mapping.split_once('=').with_context(|| {
            format!("invalid custom header mapping `{mapping}`; expected HEADER=ENV")
        })?;
        if header.trim().is_empty() || variable.trim().is_empty() {
            bail!(
                "invalid custom header mapping `{mapping}`; header and environment name are required"
            );
        }
        config
            .header_env
            .insert(header.trim().into(), variable.trim().into());
    }
    Ok(config)
}

async fn handle_auth(command: AuthCommands, settings: &mut KernexSettings) -> Result<()> {
    let manager = AuthManager::open_default()?;
    match command {
        AuthCommands::Login(args) => {
            let provider = choose_provider(args.provider.as_deref())?;
            let method = choose_auth_method(args.method)?;
            let profile = match method {
                AuthMethodArg::ApiKey => {
                    if !io::stdin().is_terminal() {
                        bail!(
                            "API key login requires an interactive terminal so the key is not exposed in arguments"
                        );
                    }
                    let key = Password::new().with_prompt("API key").interact()?;
                    manager.login_api_key(args.profile, provider, SecretValue::new(key))?
                }
                AuthMethodArg::Environment => {
                    let default = ProviderConfig::for_kind(provider, "");
                    let variable = match args.environment_variable {
                        Some(variable) => variable,
                        None if io::stdin().is_terminal() => Input::new()
                            .with_prompt("Environment variable")
                            .default(default.api_key_env.unwrap_or_default())
                            .interact_text()?,
                        None => bail!("--environment-variable is required without a terminal"),
                    };
                    manager.login_environment(args.profile, provider, variable)?
                }
                AuthMethodArg::OAuth => {
                    let client_id = args
                        .client_id
                        .or_else(|| std::env::var("GOOGLE_OAUTH_CLIENT_ID").ok())
                        .context(
                            "OAuth requires --client-id (or GOOGLE_OAUTH_CLIENT_ID for Google)",
                        )?;
                    let config = if provider == ProviderKind::Gemini
                        && args.authorization_url.is_none()
                        && args.token_url.is_none()
                    {
                        let resource_project = match args
                            .google_project
                            .or_else(|| std::env::var("GOOGLE_CLOUD_PROJECT").ok())
                        {
                            Some(project) => project,
                            None if io::stdin().is_terminal() => Input::new()
                                .with_prompt("Google Cloud project ID")
                                .interact_text()?,
                            None => bail!(
                                "Google OAuth requires --google-project or GOOGLE_CLOUD_PROJECT"
                            ),
                        };
                        OAuthConfig::google(client_id, resource_project)
                    } else if provider == ProviderKind::Custom {
                        OAuthConfig {
                            authorization_url: args.authorization_url.context(
                                "custom OAuth requires --authorization-url from the provider's official documentation",
                            )?,
                            token_url: args.token_url.context(
                                "custom OAuth requires --token-url from the provider's official documentation",
                            )?,
                            client_id,
                            scopes: args.scopes,
                            extra_authorization_parameters: Default::default(),
                            resource_project: None,
                        }
                    } else {
                        bail!(
                            "{provider} does not expose an official third-party OAuth flow configured by Kernex; use an API key or environment variable"
                        );
                    };
                    manager.login_oauth(args.profile, provider, config).await?
                }
            };
            settings.provider.name = provider;
            settings.provider.auth_profile = Some(profile.name.clone());
            settings.save_default()?;
            println!("Signed in with profile `{}` for {provider}.", profile.name);
        }
        AuthCommands::Logout { profile } => {
            let profile = select_profile(&manager, profile)?;
            manager.logout(&profile)?;
            if settings.provider.auth_profile.as_deref() == Some(&profile) {
                settings.provider.auth_profile = None;
                settings.save_default()?;
            }
            println!("Removed authentication profile `{profile}`.");
        }
        AuthCommands::Status { json } => {
            let statuses = manager.statuses()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&statuses)?);
            } else if statuses.is_empty() {
                println!("No authentication profiles configured.");
            } else {
                for status in statuses {
                    println!(
                        "{}{}\t{}\t{:?}\t{}",
                        if status.active { "* " } else { "  " },
                        status.profile.name,
                        status.profile.provider,
                        status.profile.method,
                        if status.credential_available {
                            if status.expired { "expired" } else { "ready" }
                        } else {
                            "credential unavailable"
                        }
                    );
                }
            }
        }
        AuthCommands::Use { profile } => {
            let selected = manager
                .profiles()?
                .into_iter()
                .find(|candidate| candidate.name == profile)
                .with_context(|| format!("authentication profile `{profile}` does not exist"))?;
            manager.set_active(selected.provider, &profile)?;
            settings.provider.name = selected.provider;
            settings.provider.auth_profile = Some(profile.clone());
            settings.save_default()?;
            println!("Selected authentication profile `{profile}`.");
        }
    }
    Ok(())
}

fn choose_provider(value: Option<&str>) -> Result<ProviderKind> {
    if let Some(value) = value {
        return Ok(ProviderKind::from_str(value)?);
    }
    if !io::stdin().is_terminal() {
        bail!("--provider is required without an interactive terminal");
    }
    let labels = ProviderKind::ALL.map(|provider| provider.to_string());
    let selected = Select::new()
        .with_prompt("Select provider")
        .items(&labels)
        .default(0)
        .interact()?;
    Ok(ProviderKind::ALL[selected])
}

fn choose_auth_method(value: Option<AuthMethodArg>) -> Result<AuthMethodArg> {
    if let Some(value) = value {
        return Ok(value);
    }
    if !io::stdin().is_terminal() {
        bail!("--method is required without an interactive terminal");
    }
    let selected = Select::new()
        .with_prompt("Select authentication method")
        .items([
            "Use an API key",
            "Use an environment variable",
            "Sign in with OAuth PKCE",
        ])
        .default(0)
        .interact()?;
    Ok([
        AuthMethodArg::ApiKey,
        AuthMethodArg::Environment,
        AuthMethodArg::OAuth,
    ][selected])
}

fn select_profile(manager: &AuthManager, selected: Option<String>) -> Result<String> {
    if let Some(selected) = selected {
        return Ok(selected);
    }
    let profiles = manager.profiles()?;
    if profiles.is_empty() {
        bail!("no authentication profiles are configured");
    }
    if !io::stdin().is_terminal() {
        bail!("PROFILE is required without an interactive terminal");
    }
    let labels: Vec<_> = profiles
        .iter()
        .map(|profile| format!("{} ({})", profile.name, profile.provider))
        .collect();
    let selected = Select::new()
        .with_prompt("Select profile")
        .items(&labels)
        .interact()?;
    Ok(profiles[selected].name.clone())
}

fn inspect(workspace: &Workspace, json_output: bool) -> Result<()> {
    let index = ProjectIndex::build(workspace)?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&index)?);
        return Ok(());
    }
    println!("Project: {}", index.root());
    println!("Files: {}", index.files().len());
    let languages = index.languages();
    if languages.is_empty() {
        println!("Languages: none detected");
    } else {
        println!("Languages:");
        for (language, count) in languages {
            println!("  {language}: {count}");
        }
    }
    Ok(())
}

fn search(
    workspace: &Workspace,
    query: &str,
    case_sensitive: bool,
    max_results: usize,
    json_output: bool,
) -> Result<()> {
    let index = ProjectIndex::build(workspace)?;
    let matches = index.search(workspace, query, case_sensitive, max_results)?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&matches)?);
    } else {
        for item in &matches {
            println!(
                "{}:{}:{}: {}",
                item.path, item.line, item.column, item.preview
            );
        }
        if matches.is_empty() {
            println!("No matches found.");
        }
    }
    Ok(())
}

async fn run_command(
    workspace: Arc<Workspace>,
    permissions: Arc<PermissionGate>,
    args: ExecArgs,
) -> Result<()> {
    let Some((program, command_args)) = args.command.split_first() else {
        bail!("a command program is required");
    };
    let mut spec = CommandSpec::new(program, command_args);
    spec.timeout_seconds = args.timeout;
    let output = CommandRunner::new(workspace, permissions)
        .run(&spec)
        .await?;
    print!("{}", output.stdout);
    eprint!("{}", output.stderr);
    if output.truncated {
        eprintln!("\n[kernex: output truncated]");
    }
    if !output.success {
        bail!(
            "command exited with {}",
            output
                .exit_code
                .map_or_else(|| "no status".to_owned(), |code| code.to_string())
        );
    }
    Ok(())
}

async fn ask_model(
    permissions: Arc<PermissionGate>,
    config: ProviderConfig,
    prompt: String,
) -> Result<()> {
    let provider = prepare_http_provider(config, permissions).await?;
    let response = provider
        .complete(CompletionRequest::new(vec![
            Message::new(Role::System, agent_core::SYSTEM_PROMPT),
            Message::new(Role::User, prompt),
        ]))
        .await?;
    print_markdown(&response.content);
    Ok(())
}

fn review(workspace: Arc<Workspace>, permissions: Arc<PermissionGate>) -> Result<()> {
    let git = GitRepository::new(workspace, permissions);
    println!("Status:\n{}", git.status()?.stdout);
    let diff = git.diff(false)?.stdout;
    if diff.is_empty() {
        println!("No working-tree changes.");
    } else {
        println!("Diff:\n{diff}");
    }
    Ok(())
}

fn initialize_instructions(workspace: &Workspace, force: bool) -> Result<()> {
    let path = workspace.root().join("KERNEX.md");
    if path.exists()
        && !force
        && (!io::stdin().is_terminal()
            || !Confirm::new()
                .with_prompt("KERNEX.md already exists. Overwrite it?")
                .default(false)
                .interact()?)
    {
        bail!("KERNEX.md was not changed");
    }
    std::fs::write(
        &path,
        "# Kernex project instructions\n\nDescribe the repository conventions, validation commands, and safety constraints that Kernex should follow.\n",
    )
    .with_context(|| format!("could not write {}", path.display()))?;
    println!("Created {}.", path.display());
    Ok(())
}

fn load_session(workspace: &Workspace, id: Option<&str>) -> Result<SessionRecord> {
    let store = SessionStore::open_default()?;
    match id {
        Some(id) => store
            .get(id)?
            .with_context(|| format!("session `{id}` does not exist")),
        None => store
            .latest_for_workspace(&workspace.root().display().to_string())?
            .context("this project has no stored sessions"),
    }
}

fn list_sessions(workspace: &Workspace, limit: usize, json: bool) -> Result<()> {
    let sessions = SessionStore::open_default()?
        .list_for_workspace(&workspace.root().display().to_string(), limit)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
    } else if sessions.is_empty() {
        println!("No sessions stored for this project.");
    } else {
        for session in sessions {
            println!(
                "{}\t{}\t{}/{}\t{:?}",
                session.id, session.updated_at, session.provider, session.model, session.status
            );
        }
    }
    Ok(())
}

async fn list_models_and_providers(
    settings: &KernexSettings,
    json: bool,
    discover: bool,
    assume_yes: bool,
) -> Result<()> {
    let configs: Vec<_> = ProviderKind::ALL
        .into_iter()
        .map(|kind| ProviderConfig::for_kind(kind, ""))
        .collect();
    let models = if discover {
        let mut config = settings.provider.to_provider_config();
        if config.model.trim().is_empty() {
            config.model = "model-catalog".into();
        }
        Some(
            prepare_http_provider(config, basic_permissions(assume_yes))
                .await?
                .models()
                .await?,
        )
    } else {
        None
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "selected_provider": settings.provider.name,
                "selected_model": settings.provider.model,
                "providers": configs,
                "models": models,
            }))?
        );
    } else {
        println!(
            "Selected: {}/{}",
            settings.provider.name,
            if settings.provider.model.is_empty() {
                "<model not configured>"
            } else {
                &settings.provider.model
            }
        );
        println!("PROVIDER             DEFAULT API BASE                                 AUTH");
        for config in configs {
            println!(
                "{:<20} {:<48} {}",
                config.kind,
                if config.base_url.is_empty() {
                    "<required>"
                } else {
                    &config.base_url
                },
                config.api_key_env.as_deref().unwrap_or("<none>")
            );
        }
        if let Some(models) = models {
            println!("\nDiscovered models:");
            for model in models {
                println!(
                    "  {}{}",
                    model.id,
                    model
                        .display_name
                        .as_deref()
                        .map(|name| format!(" ({name})"))
                        .unwrap_or_default()
                );
            }
        } else {
            println!("\nUse `kernex models --discover` to query the selected provider.");
        }
    }
    Ok(())
}

fn list_extensions(workspace: &Workspace) -> Result<()> {
    let plugins = PluginRegistry::discover(workspace)?;
    let config = KernexConfig::load(workspace)?;
    println!("Plugins: {}", plugins.plugins().len());
    for plugin in plugins.plugins() {
        println!(
            "  {} {} ({} tool(s), {})",
            plugin.name, plugin.version, plugin.tool_count, plugin.path
        );
    }
    println!("MCP servers: {}", config.mcp_servers.len());
    for server in config.mcp_servers {
        println!(
            "  {}: {} {}",
            server.name,
            server.command,
            server.args.join(" ")
        );
    }
    println!("Language servers: {}", config.language_servers.len());
    for entry in config.language_servers {
        println!(
            "  {}: {} {} [{}]",
            entry.server.language_id,
            entry.server.command,
            entry.server.args.join(" "),
            entry.extensions.join(", ")
        );
    }
    Ok(())
}

fn print_markdown(markdown: &str) {
    if io::stdout().is_terminal() {
        termimad::print_text(markdown);
    } else {
        println!("{markdown}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn direct_task_is_accepted_without_a_subcommand() {
        let cli = Cli::try_parse_from(["kernex", "Fix the tests"]).unwrap();
        assert_eq!(cli.task.as_deref(), Some("Fix the tests"));
        assert!(cli.command.is_none());
    }

    #[test]
    fn run_subcommand_remains_backward_compatible() {
        let cli = Cli::try_parse_from([
            "kernex",
            "run",
            "--provider",
            "local",
            "--model",
            "test-model",
            "Inspect the project",
        ])
        .unwrap();
        assert!(matches!(cli.command, Some(Commands::Run(_))));
    }
}
