use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};

use agent_core::{
    AgentEngine, AgentEvent, Approver, EventSink, HttpModelProvider, KernexConfig,
    PermissionDecision, PermissionGate, PermissionPolicy, PermissionRequest, ProviderConfig,
    ProviderKind, Toolbox, Workspace,
};
use eframe::egui::{self, Color32, RichText};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Kernex")
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([820.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Kernex",
        options,
        Box::new(|context| Ok(Box::new(KernexApp::new(context)))),
    )
}

#[derive(Clone)]
struct PendingApproval {
    id: u64,
    request: PermissionRequest,
}

#[derive(Default)]
struct ApprovalState {
    next_id: AtomicU64,
    pending: Mutex<VecDeque<PendingApproval>>,
    responses: Mutex<HashMap<u64, PermissionDecision>>,
    response_ready: Condvar,
}

impl ApprovalState {
    fn request(&self, request: &PermissionRequest) -> PermissionDecision {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut pending) = self.pending.lock() {
            pending.push_back(PendingApproval {
                id,
                request: request.clone(),
            });
        } else {
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

    fn front(&self) -> Option<PendingApproval> {
        self.pending.lock().ok()?.front().cloned()
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
    state: Arc<ApprovalState>,
}

impl Approver for DesktopApprover {
    fn decide(&self, request: &PermissionRequest) -> PermissionDecision {
        self.state.request(request)
    }
}

enum UiMessage {
    Event(AgentEvent),
    Finished(Result<String, String>),
}

struct ChannelEvents {
    sender: mpsc::Sender<UiMessage>,
    context: egui::Context,
}

impl EventSink for ChannelEvents {
    fn emit(&self, event: AgentEvent) {
        let _ = self.sender.send(UiMessage::Event(event));
        self.context.request_repaint();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputTab {
    Activity,
    Diff,
    Result,
}

struct KernexApp {
    project: String,
    provider: ProviderKind,
    model: String,
    base_url: String,
    api_key_env: String,
    header_env: String,
    task: String,
    max_steps: usize,
    running: bool,
    cancelled: Arc<AtomicBool>,
    approvals: Arc<ApprovalState>,
    sender: mpsc::Sender<UiMessage>,
    receiver: mpsc::Receiver<UiMessage>,
    activity: Vec<String>,
    diffs: Vec<String>,
    result: String,
    error: Option<String>,
    tab: OutputTab,
}

impl KernexApp {
    fn new(_context: &eframe::CreationContext<'_>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let config = ProviderConfig::for_kind(ProviderKind::OpenAiCompatible, "");
        Self {
            project: ".".into(),
            provider: config.kind,
            model: String::new(),
            base_url: config.base_url,
            api_key_env: config.api_key_env.unwrap_or_default(),
            header_env: String::new(),
            task: String::new(),
            max_steps: 24,
            running: false,
            cancelled: Arc::new(AtomicBool::new(false)),
            approvals: Arc::new(ApprovalState::default()),
            sender,
            receiver,
            activity: Vec::new(),
            diffs: Vec::new(),
            result: String::new(),
            error: None,
            tab: OutputTab::Activity,
        }
    }

    fn change_provider(&mut self, provider: ProviderKind) {
        let defaults = ProviderConfig::for_kind(provider, "");
        self.provider = provider;
        self.base_url = defaults.base_url;
        self.api_key_env = defaults.api_key_env.unwrap_or_default();
        self.header_env.clear();
    }

    fn start(&mut self, context: &egui::Context) {
        self.activity.clear();
        self.diffs.clear();
        self.result.clear();
        self.error = None;
        self.tab = OutputTab::Activity;
        self.running = true;
        self.cancelled = Arc::new(AtomicBool::new(false));
        self.approvals = Arc::new(ApprovalState::default());

        let project = self.project.clone();
        let provider_kind = self.provider;
        let model = self.model.clone();
        let base_url = self.base_url.clone();
        let api_key_env = self.api_key_env.clone();
        let header_env = self.header_env.clone();
        let task = self.task.clone();
        let max_steps = self.max_steps;
        let sender = self.sender.clone();
        let events = Arc::new(ChannelEvents {
            sender: sender.clone(),
            context: context.clone(),
        });
        let approvals = self.approvals.clone();
        let cancelled = self.cancelled.clone();
        let repaint = context.clone();

        std::thread::spawn(move || {
            let outcome = run_agent(DesktopRun {
                project,
                provider_kind,
                model,
                base_url,
                api_key_env,
                header_env,
                task,
                max_steps,
                approvals,
                cancelled,
                events,
            });
            let _ = sender.send(UiMessage::Finished(outcome));
            repaint.request_repaint();
        });
    }

    fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.approvals.deny_all();
        self.activity.push("Cancellation requested by user.".into());
    }

    fn drain_messages(&mut self) {
        while let Ok(message) = self.receiver.try_recv() {
            match message {
                UiMessage::Event(event) => self.record_event(event),
                UiMessage::Finished(outcome) => {
                    self.running = false;
                    match outcome {
                        Ok(answer) => {
                            self.result = answer;
                            self.tab = OutputTab::Result;
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
            }
        }
    }

    fn record_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Started {
                provider, model, ..
            } => self
                .activity
                .push(format!("Started agent with {provider}/{model}.")),
            AgentEvent::ModelRequested { step } => self
                .activity
                .push(format!("Step {step}: requested model response.")),
            AgentEvent::ModelResponded {
                step,
                content,
                tool_calls,
            } => {
                if !content.is_empty() {
                    self.activity
                        .push(format!("Step {step}: model said\n{content}"));
                }
                for call in tool_calls {
                    self.activity.push(format!(
                        "Step {step}: requested {} {}",
                        call.name, call.arguments
                    ));
                }
            }
            AgentEvent::ToolStarted { call, .. } => self
                .activity
                .push(format!("Running {} {}", call.name, call.arguments)),
            AgentEvent::ToolFinished {
                name, result, diff, ..
            } => {
                let preview: String = result.chars().take(4_000).collect();
                let suffix = if result.chars().count() > 4_000 {
                    "\n[activity preview truncated]"
                } else {
                    ""
                };
                self.activity
                    .push(format!("{name} completed.\n{preview}{suffix}"));
                if let Some(diff) = diff {
                    self.diffs.push(diff);
                }
            }
            AgentEvent::ToolFailed { name, error, .. } => {
                self.activity.push(format!("{name} failed: {error}"));
            }
            AgentEvent::Completed { steps } => self
                .activity
                .push(format!("Completed after {steps} step(s).")),
        }
    }

    fn configuration_panel(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        ui.heading("Task");
        ui.label("Project directory");
        ui.text_edit_singleline(&mut self.project);

        ui.add_space(8.0);
        ui.label("Provider");
        let old_provider = self.provider;
        egui::ComboBox::from_id_salt("provider")
            .selected_text(self.provider.to_string())
            .show_ui(ui, |ui| {
                for provider in ProviderKind::ALL {
                    ui.selectable_value(&mut self.provider, provider, provider.to_string());
                }
            });
        if old_provider != self.provider {
            self.change_provider(self.provider);
        }

        ui.label("Model");
        ui.text_edit_singleline(&mut self.model);
        ui.label("API base URL");
        ui.text_edit_singleline(&mut self.base_url);
        ui.label("API key environment variable");
        ui.text_edit_singleline(&mut self.api_key_env);
        ui.label("Custom header environment mappings");
        ui.add(
            egui::TextEdit::multiline(&mut self.header_env)
                .desired_rows(2)
                .hint_text("X-API-Key=SERVICE_KEY"),
        );
        ui.small("Kernex reads the key only for the request and never stores its value.");

        ui.add_space(8.0);
        ui.label("Development request");
        ui.add(
            egui::TextEdit::multiline(&mut self.task)
                .desired_rows(9)
                .hint_text("Explain the change, bug, or investigation..."),
        );
        ui.horizontal(|ui| {
            ui.label("Maximum steps");
            ui.add(egui::DragValue::new(&mut self.max_steps).range(1..=100));
        });

        let ready = !self.running
            && !self.project.trim().is_empty()
            && !self.model.trim().is_empty()
            && !self.base_url.trim().is_empty()
            && !self.task.trim().is_empty();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(ready, egui::Button::new("Run agent"))
                .clicked()
            {
                self.start(context);
            }
            if ui
                .add_enabled(self.running, egui::Button::new("Cancel"))
                .clicked()
            {
                self.cancel();
            }
        });
        if self.running {
            ui.spinner();
            ui.label("Agent is working. Protected actions pause for approval.");
        }
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
    }

    fn approval_panel(&mut self, ui: &mut egui::Ui) {
        let Some(pending) = self.approvals.front() else {
            return;
        };
        ui.group(|ui| {
            ui.heading(RichText::new("Permission required").color(Color32::YELLOW));
            ui.label(format!(
                "{} · {:?}",
                pending.request.capability, pending.request.risk
            ));
            ui.strong(&pending.request.summary);
            ui.monospace(&pending.request.resource);
            for detail in &pending.request.details {
                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .show(ui, |ui| {
                        ui.monospace(detail);
                    });
            }
            ui.horizontal(|ui| {
                if ui.button("Allow once").clicked() {
                    self.approvals
                        .respond(pending.id, PermissionDecision::AllowOnce);
                }
                if ui.button("Allow for session").clicked() {
                    self.approvals
                        .respond(pending.id, PermissionDecision::AllowForSession);
                }
                if ui.button("Deny").clicked() {
                    self.approvals.respond(pending.id, PermissionDecision::Deny);
                }
            });
        });
    }

    fn output_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, OutputTab::Activity, "Activity");
            ui.selectable_value(
                &mut self.tab,
                OutputTab::Diff,
                format!("Diffs ({})", self.diffs.len()),
            );
            ui.selectable_value(&mut self.tab, OutputTab::Result, "Result");
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(self.tab == OutputTab::Activity)
            .show(ui, |ui| match self.tab {
                OutputTab::Activity => {
                    for line in &self.activity {
                        ui.monospace(line);
                        ui.separator();
                    }
                }
                OutputTab::Diff => {
                    if self.diffs.is_empty() {
                        ui.label("No file modifications have been applied.");
                    }
                    for diff in &self.diffs {
                        ui.monospace(diff);
                        ui.separator();
                    }
                }
                OutputTab::Result => {
                    if self.result.is_empty() {
                        ui.label("The agent has not returned a final answer yet.");
                    } else {
                        ui.label(&self.result);
                    }
                }
            });
    }
}

impl eframe::App for KernexApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_messages();
        if self.running {
            context.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = root.ctx().clone();
        egui::CentralPanel::default().show(root, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Kernex");
                ui.label("Native, safe, provider-independent coding agent");
            });
            ui.separator();
            ui.columns(2, |columns| {
                let (left, right) = columns.split_at_mut(1);
                egui::ScrollArea::vertical().show(&mut left[0], |ui| {
                    self.configuration_panel(ui, &context);
                });
                self.approval_panel(&mut right[0]);
                self.output_panel(&mut right[0]);
            });
        });
    }
}

impl Drop for KernexApp {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.approvals.deny_all();
    }
}

struct DesktopRun {
    project: String,
    provider_kind: ProviderKind,
    model: String,
    base_url: String,
    api_key_env: String,
    header_env: String,
    task: String,
    max_steps: usize,
    approvals: Arc<ApprovalState>,
    cancelled: Arc<AtomicBool>,
    events: Arc<dyn EventSink>,
}

fn run_agent(run: DesktopRun) -> Result<String, String> {
    let workspace = Arc::new(Workspace::open(&run.project).map_err(|error| error.to_string())?);
    let approver = Arc::new(DesktopApprover {
        state: run.approvals,
    });
    let permissions = Arc::new(PermissionGate::new(
        PermissionPolicy::default(),
        Some(approver),
    ));
    let mut config = ProviderConfig::for_kind(run.provider_kind, run.model);
    config.base_url = run.base_url;
    config.api_key_env = (!run.api_key_env.trim().is_empty()).then_some(run.api_key_env);
    for mapping in run
        .header_env
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let (header, variable) = mapping.split_once('=').ok_or_else(|| {
            format!("invalid custom header mapping `{mapping}`; expected HEADER=ENV")
        })?;
        if header.trim().is_empty() || variable.trim().is_empty() {
            return Err(format!(
                "invalid custom header mapping `{mapping}`; header and environment name are required"
            ));
        }
        config
            .header_env
            .insert(header.trim().to_owned(), variable.trim().to_owned());
    }
    let provider = Arc::new(
        HttpModelProvider::new(config, permissions.clone()).map_err(|error| error.to_string())?,
    );
    let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
    runtime
        .block_on(async move {
            let configuration =
                KernexConfig::load(&workspace).map_err(|error| error.to_string())?;
            let language_servers = configuration.language_servers;
            let toolbox = Toolbox::new(workspace, permissions)
                .map_err(|error| error.to_string())?
                .connect_mcp(configuration.mcp_servers)
                .await
                .map_err(|error| error.to_string())?
                .connect_language_servers(language_servers)
                .await
                .map_err(|error| error.to_string())?;
            let engine = AgentEngine::new(provider, toolbox, run.events)
                .with_max_steps(run.max_steps)
                .with_cancellation(run.cancelled);
            engine
                .run(run.task)
                .await
                .map_err(|error| error.to_string())
        })
        .map(|result| result.final_answer)
}
