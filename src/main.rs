use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use agent_core::{
    AgentEngine, AgentEvent, Approver, CommandRunner, CommandSpec, CompletionRequest, EventSink,
    GitRepository, HttpModelProvider, KernexConfig, Message, ModelProvider, PermissionDecision,
    PermissionGate, PermissionPolicy, PermissionRequest, PluginRegistry, ProjectIndex,
    ProviderConfig, ProviderKind, Role, SYSTEM_PROMPT, SyntaxAnalyzer, Toolbox, VERSION, Workspace,
};
use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "kernex",
    version = VERSION,
    about = "A safe, provider-independent AI coding agent"
)]
struct Cli {
    /// Project directory Kernex is allowed to access.
    #[arg(short = 'C', long, default_value = ".", global = true)]
    project: PathBuf,

    /// Approve protected actions for this invocation without prompting.
    #[arg(long, global = true)]
    yes: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Summarize files and detected languages in a project.
    Inspect {
        /// Emit machine-readable JSON.
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
        /// Review staged changes instead of unstaged changes.
        #[arg(long)]
        staged: bool,
    },
    /// Execute one argument-vector command after permission review.
    Exec(ExecArgs),
    /// Send a single prompt to a configured model provider.
    Ask(AskArgs),
    /// Run an agent task with project tools and transparent permission prompts.
    Run(RunArgs),
    /// List supported provider adapters and their safe defaults.
    Providers {
        #[arg(long)]
        json: bool,
    },
    /// Show discovered plugins and configured MCP/LSP servers without starting them.
    Extensions,
}

#[derive(Debug, Args)]
struct ExecArgs {
    /// Maximum command runtime in seconds.
    #[arg(long, default_value_t = 120)]
    timeout: u64,

    /// Program and arguments. Use `--` before arguments beginning with `-`.
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<String>,
}

#[derive(Debug, Args)]
struct AskArgs {
    /// Provider adapter: openai-compatible, anthropic, gemini, local, or custom.
    #[arg(long, default_value = "openai-compatible")]
    provider: String,

    /// Provider model identifier.
    #[arg(long)]
    model: String,

    /// Override the provider's API base URL.
    #[arg(long)]
    base_url: Option<String>,

    /// Name of the environment variable holding the API key; its value is never persisted.
    #[arg(long)]
    api_key_env: Option<String>,

    /// Custom header mapped to an environment variable, for example `X-API-Key=SERVICE_KEY`.
    #[arg(long = "header-env", value_name = "HEADER=ENV")]
    header_env: Vec<String>,

    /// User request sent to the model.
    prompt: String,
}

#[derive(Debug, Args)]
struct RunArgs {
    /// Provider adapter: openai-compatible, anthropic, gemini, local, or custom.
    #[arg(long, default_value = "openai-compatible")]
    provider: String,

    /// Provider model identifier.
    #[arg(long)]
    model: String,

    /// Override the provider's API base URL.
    #[arg(long)]
    base_url: Option<String>,

    /// Name of the environment variable holding the API key; its value is never persisted.
    #[arg(long)]
    api_key_env: Option<String>,

    /// Custom header mapped to an environment variable, for example `X-API-Key=SERVICE_KEY`.
    #[arg(long = "header-env", value_name = "HEADER=ENV")]
    header_env: Vec<String>,

    /// Maximum model/tool iterations before Kernex stops safely.
    #[arg(long, default_value_t = 24)]
    max_steps: usize,

    /// Development task for the agent to complete.
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

        eprint!("Allow? [y]es / [s]ession / [N]o: ");
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
            } => eprintln!("[kernex] agent started with {provider}/{model}"),
            AgentEvent::ModelRequested { step } => {
                eprintln!("[kernex] step {step}: requesting model response")
            }
            AgentEvent::ModelResponded {
                step, tool_calls, ..
            } => {
                if !tool_calls.is_empty() {
                    eprintln!(
                        "[kernex] step {step}: model requested {} tool action(s)",
                        tool_calls.len()
                    );
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
    let workspace = Arc::new(
        Workspace::open(&cli.project)
            .with_context(|| format!("cannot open project {}", cli.project.display()))?,
    );
    let approver = Arc::new(TerminalApprover {
        assume_yes: cli.yes,
    });
    let permissions = Arc::new(PermissionGate::new(
        PermissionPolicy::default(),
        Some(approver),
    ));

    match cli.command {
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
            let output = GitRepository::new(workspace, permissions).status()?;
            print!("{}", output.stdout);
        }
        Commands::Diff { staged } => {
            let output = GitRepository::new(workspace, permissions).diff(staged)?;
            print!("{}", output.stdout);
        }
        Commands::Exec(args) => run_command(workspace, permissions, args).await?,
        Commands::Ask(args) => ask_model(permissions, args).await?,
        Commands::Run(args) => run_agent(workspace, permissions, args).await?,
        Commands::Providers { json } => list_providers(json)?,
        Commands::Extensions => list_extensions(&workspace)?,
    }
    Ok(())
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

async fn ask_model(permissions: Arc<PermissionGate>, args: AskArgs) -> Result<()> {
    let provider = configured_provider(
        permissions,
        &args.provider,
        args.model,
        args.base_url,
        args.api_key_env,
        args.header_env,
    )?;
    let response = provider
        .complete(CompletionRequest::new(vec![
            Message::new(Role::System, SYSTEM_PROMPT),
            Message::new(Role::User, args.prompt),
        ]))
        .await?;
    println!("{}", response.content);
    if response.usage.input_tokens.is_some() || response.usage.output_tokens.is_some() {
        eprintln!(
            "[tokens: input={}, output={}]",
            response
                .usage
                .input_tokens
                .map_or_else(|| "unknown".to_owned(), |count| count.to_string()),
            response
                .usage
                .output_tokens
                .map_or_else(|| "unknown".to_owned(), |count| count.to_string())
        );
    }
    Ok(())
}

async fn run_agent(
    workspace: Arc<Workspace>,
    permissions: Arc<PermissionGate>,
    args: RunArgs,
) -> Result<()> {
    let provider = Arc::new(configured_provider(
        permissions.clone(),
        &args.provider,
        args.model,
        args.base_url,
        args.api_key_env,
        args.header_env,
    )?);
    let configuration = KernexConfig::load(&workspace)?;
    let language_servers = configuration.language_servers;
    let toolbox = Toolbox::new(workspace, permissions)?
        .connect_mcp(configuration.mcp_servers)
        .await?
        .connect_language_servers(language_servers)
        .await?;
    let engine = AgentEngine::new(provider, toolbox, Arc::new(TerminalEvents))
        .with_max_steps(args.max_steps);
    let result = engine.run(args.task).await?;
    println!("{}", result.final_answer);
    Ok(())
}

fn configured_provider(
    permissions: Arc<PermissionGate>,
    provider: &str,
    model: String,
    base_url: Option<String>,
    api_key_env: Option<String>,
    header_env: Vec<String>,
) -> Result<HttpModelProvider> {
    let kind = ProviderKind::from_str(provider)?;
    let mut config = ProviderConfig::for_kind(kind, model);
    if let Some(base_url) = base_url {
        config.base_url = base_url;
    }
    if let Some(api_key_env) = api_key_env {
        config.api_key_env = (!api_key_env.is_empty()).then_some(api_key_env);
    }
    for mapping in header_env {
        let Some((header, variable)) = mapping.split_once('=') else {
            bail!("invalid --header-env `{mapping}`; expected HEADER=ENV");
        };
        if header.trim().is_empty() || variable.trim().is_empty() {
            bail!("invalid --header-env `{mapping}`; header and environment name are required");
        }
        config
            .header_env
            .insert(header.trim().to_owned(), variable.trim().to_owned());
    }
    Ok(HttpModelProvider::new(config, permissions)?)
}

fn list_providers(json_output: bool) -> Result<()> {
    let configs: Vec<_> = ProviderKind::ALL
        .into_iter()
        .map(|kind| ProviderConfig::for_kind(kind, "<model>"))
        .collect();
    if json_output {
        println!("{}", serde_json::to_string_pretty(&configs)?);
    } else {
        println!(
            "PROVIDER             DEFAULT BASE URL                                 API KEY ENV"
        );
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
