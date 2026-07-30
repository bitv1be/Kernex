use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use kernex_agent_core::{
    AgentEvent, Approver, CodexAppServerClient, CodexAppServerConfig, CodexTurnConfig, EventSink,
    PermissionDecision, PermissionMode, PermissionRequest, ProviderKind, ProviderStreamEvent,
    run_codex_turn_with_server,
};

fn fixture_config() -> CodexAppServerConfig {
    CodexAppServerConfig {
        executable: PathBuf::from(env!("CARGO_BIN_EXE_kernex-protocol-fixture")),
        arguments: vec!["codex".into()],
    }
}

struct AllowForSession {
    requests: Arc<Mutex<Vec<PermissionRequest>>>,
}

impl Approver for AllowForSession {
    fn decide(&self, request: &PermissionRequest) -> PermissionDecision {
        self.requests.lock().unwrap().push(request.clone());
        PermissionDecision::AllowForSession
    }
}

#[derive(Default)]
struct RecordedEvents(Mutex<Vec<AgentEvent>>);

impl EventSink for RecordedEvents {
    fn emit(&self, event: AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[tokio::test]
async fn account_models_and_subscription_limits_round_trip() {
    let mut client = CodexAppServerClient::connect_with(fixture_config())
        .await
        .unwrap();
    let account = client.account(false).await.unwrap();
    assert_eq!(account.account.unwrap().plan_type.as_deref(), Some("plus"));

    let models = client.models().await.unwrap();
    assert_eq!(models[0].id, "gpt-fixture");
    assert!(models[0].is_default);

    let limits = client.rate_limits().await.unwrap();
    assert_eq!(
        limits.rate_limits.unwrap().primary.unwrap().used_percent,
        25
    );
    assert_eq!(limits.rate_limit_reset_credits.unwrap().available_count, 2);
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn managed_chatgpt_login_waits_for_completion_notification() {
    let mut client = CodexAppServerClient::connect_with(fixture_config())
        .await
        .unwrap();
    let login = client.start_chatgpt_login().await.unwrap();
    assert_eq!(login.login_id, "login-fixture");
    assert_eq!(login.auth_url, "https://example.com/login");

    let account = client.wait_for_login(&login.login_id).await.unwrap();
    let account = account.account.unwrap();
    assert_eq!(account.account_type, "chatgpt");
    assert_eq!(account.plan_type.as_deref(), Some("plus"));
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn agent_turn_persists_thread_and_bridges_approvals() {
    let workspace =
        std::env::temp_dir().join(format!("kernex-codex-app-server-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&workspace).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(RecordedEvents::default());
    let result = run_codex_turn_with_server(
        fixture_config(),
        CodexTurnConfig {
            workspace: workspace.clone(),
            model: "gpt-fixture".into(),
            permission_mode: PermissionMode::AutoSafe,
            provider_thread_id: None,
        },
        "Run the tests",
        Vec::new(),
        Arc::new(AllowForSession {
            requests: requests.clone(),
        }),
        events.clone(),
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();

    assert_eq!(result.final_answer, "Fixture complete");
    assert_eq!(result.provider_thread_id.as_deref(), Some("thread-fixture"));
    assert_eq!(result.token_usage.input_tokens, Some(7));
    assert_eq!(result.token_usage.output_tokens, Some(3));
    assert_eq!(requests.lock().unwrap()[0].resource, "cargo test");
    let events = events.0.lock().unwrap();
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::Started { provider, .. } if provider == &ProviderKind::Codex.to_string()
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::ModelDelta {
            event: ProviderStreamEvent::TextDelta { text },
            ..
        } if text == "Fixture complete"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::ToolFinished { result, .. } if result == "tests passed"
    )));
    drop(events);
    fs::remove_dir_all(workspace).unwrap();
}
