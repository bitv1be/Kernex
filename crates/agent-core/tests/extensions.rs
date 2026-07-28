use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use kernex_agent_core::{
    Approver, CompletionRequest, HttpModelProvider, LanguageServerConfig, LspClient, McpClient,
    McpServerConfig, Message, ModelProvider, PermissionDecision, PermissionGate, PermissionPolicy,
    PermissionRequest, ProviderConfig, ProviderKind, Role, ToolCall, Toolbox, Workspace,
};
use serde_json::json;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

struct AllowAll;

impl Approver for AllowAll {
    fn decide(&self, _request: &PermissionRequest) -> PermissionDecision {
        PermissionDecision::AllowForSession
    }
}

fn permissions() -> Arc<PermissionGate> {
    Arc::new(PermissionGate::new(
        PermissionPolicy::default(),
        Some(Arc::new(AllowAll)),
    ))
}

fn fixture() -> String {
    env!("CARGO_BIN_EXE_kernex-protocol-fixture").to_owned()
}

fn temp_workspace(label: &str) -> PathBuf {
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("kernex-{label}-{}-{id}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn cleanup(path: &Path) {
    fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn mcp_stdio_round_trip() {
    let root = temp_workspace("mcp");
    let workspace = Arc::new(Workspace::open(&root).unwrap());
    let mut client = McpClient::connect(
        workspace,
        permissions(),
        McpServerConfig {
            name: "fixture".into(),
            command: fixture(),
            args: vec!["mcp".into()],
            env: BTreeMap::new(),
        },
    )
    .await
    .unwrap();
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools[0].name, "echo");
    let response = client
        .call_tool("echo", json!({"value": "hello"}))
        .await
        .unwrap();
    assert_eq!(response["structuredContent"]["value"], "hello");
    client.terminate().await.unwrap();
    cleanup(&root);
}

#[tokio::test]
async fn openai_compatible_http_round_trip() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("local sockets are disabled; HTTP round-trip is covered on native CI");
            return;
        }
        Err(error) => panic!("could not bind provider fixture: {error}"),
    };
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let count = stream.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
            let Some(header_end) = request.windows(4).position(|item| item == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
        let request = String::from_utf8_lossy(&request);
        assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(request.contains("\"model\":\"fixture-model\""));
        let body = r#"{"model":"fixture-model","choices":[{"message":{"content":"fixture response"}}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        stream.flush().unwrap();
    });

    let mut config = ProviderConfig::for_kind(ProviderKind::Local, "fixture-model");
    config.base_url = format!("http://{address}/v1");
    let provider = HttpModelProvider::new(config, permissions()).unwrap();
    let response = provider
        .complete(CompletionRequest::new(vec![Message::new(
            Role::User,
            "hello",
        )]))
        .await
        .unwrap();
    assert_eq!(response.content, "fixture response");
    assert_eq!(response.usage.output_tokens, Some(2));
    server.join().unwrap();
}

#[tokio::test]
async fn lsp_content_length_round_trip() {
    let root = temp_workspace("lsp");
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn sample() {}\n").unwrap();
    let workspace = Arc::new(Workspace::open(&root).unwrap());
    let mut client = LspClient::connect(
        workspace,
        permissions(),
        LanguageServerConfig {
            language_id: "rust".into(),
            command: fixture(),
            args: vec!["lsp".into()],
            env: BTreeMap::new(),
        },
    )
    .await
    .unwrap();
    let symbols = client.document_symbols("src/lib.rs").await.unwrap();
    assert_eq!(symbols[0]["name"], "FixtureSymbol");
    client.terminate().await.unwrap();
    cleanup(&root);
}

#[tokio::test]
async fn process_plugin_is_discovered_and_executed() {
    let root = temp_workspace("plugin");
    let plugin_dir = root.join(".kernex/plugins/fixture");
    fs::create_dir_all(&plugin_dir).unwrap();
    let executable = format!("{:?}", fixture());
    fs::write(
        plugin_dir.join("plugin.toml"),
        format!(
            r#"[plugin]
id = "fixture"
name = "Fixture"
version = "0.1.0"

[[tools]]
name = "echo"
description = "Echo input"
command = {executable}
args = ["plugin"]
input_schema = '{{"type":"object"}}'
"#
        ),
    )
    .unwrap();
    let workspace = Arc::new(Workspace::open(&root).unwrap());
    let toolbox = Toolbox::new(workspace, permissions()).unwrap();
    assert!(
        toolbox
            .definitions()
            .iter()
            .any(|tool| tool.name == "plugin__fixture__echo")
    );
    let output = toolbox
        .execute(&ToolCall {
            id: "plugin-call".into(),
            name: "plugin__fixture__echo".into(),
            arguments: json!({"value": 7}),
        })
        .await
        .unwrap();
    let response: serde_json::Value = serde_json::from_str(output.content.trim()).unwrap();
    assert_eq!(response["received"]["value"], 7);
    cleanup(&root);
}

#[tokio::test]
async fn file_tools_return_reviewable_diffs() {
    let root = temp_workspace("edits");
    let workspace = Arc::new(Workspace::open(&root).unwrap());
    let toolbox = Toolbox::new(workspace, permissions()).unwrap();
    let created = toolbox
        .execute(&ToolCall {
            id: "write".into(),
            name: "write_file".into(),
            arguments: json!({"path": "note.txt", "content": "before\n"}),
        })
        .await
        .unwrap();
    assert!(created.diff.unwrap().contains("+before"));
    let listed = toolbox
        .execute(&ToolCall {
            id: "list".into(),
            name: "list_files".into(),
            arguments: json!({"max_files": 1}),
        })
        .await
        .unwrap();
    let listed: serde_json::Value = serde_json::from_str(&listed.content).unwrap();
    assert_eq!(listed["total_files"], 1);
    assert_eq!(listed["files"][0]["path"], "note.txt");
    let replaced = toolbox
        .execute(&ToolCall {
            id: "replace".into(),
            name: "replace_in_file".into(),
            arguments: json!({"path": "note.txt", "old": "before", "new": "after"}),
        })
        .await
        .unwrap();
    let diff = replaced.diff.unwrap();
    assert!(diff.contains("-before"));
    assert!(diff.contains("+after"));
    assert_eq!(
        fs::read_to_string(root.join("note.txt")).unwrap(),
        "after\n"
    );

    toolbox
        .execute(&ToolCall {
            id: "write-second".into(),
            name: "write_file".into(),
            arguments: json!({"path": "second.txt", "content": "second\n"}),
        })
        .await
        .unwrap();
    let listed = toolbox
        .execute(&ToolCall {
            id: "list-second".into(),
            name: "list_files".into(),
            arguments: json!({"max_files": 1}),
        })
        .await
        .unwrap();
    let listed: serde_json::Value = serde_json::from_str(&listed.content).unwrap();
    assert_eq!(listed["total_files"], 2);
    assert_eq!(listed["truncated"], true);
    cleanup(&root);
}
