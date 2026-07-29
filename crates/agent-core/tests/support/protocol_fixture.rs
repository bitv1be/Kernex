use std::io::{self, BufRead, Read, Write};

use serde_json::{Value, json};

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("mcp") => serve_mcp(),
        Some("lsp") => serve_lsp(),
        Some("plugin") => serve_plugin(),
        Some("codex") => serve_codex(),
        _ => std::process::exit(2),
    }
}

fn serve_mcp() {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = message.get("id") else {
            continue;
        };
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let result = match method {
            "initialize" => json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "fixture", "version": "0.1.0"}
            }),
            "tools/list" => json!({
                "tools": [{
                    "name": "echo",
                    "title": "Echo",
                    "description": "Return the supplied input",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"value": {}},
                        "required": ["value"]
                    },
                    "annotations": {"readOnlyHint": true}
                }]
            }),
            "tools/call" => {
                let arguments = message
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or(Value::Null);
                json!({
                    "content": [{"type": "text", "text": arguments.to_string()}],
                    "structuredContent": arguments,
                    "isError": false
                })
            }
            _ => {
                write_json_line(
                    &mut stdout,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {"code": -32601, "message": "unknown fixture method"}
                    }),
                );
                continue;
            }
        };
        write_json_line(
            &mut stdout,
            &json!({"jsonrpc": "2.0", "id": id, "result": result}),
        );
    }
}

fn serve_lsp() {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut stdout = io::stdout().lock();
    while let Some(message) = read_lsp_message(&mut reader) {
        let Some(id) = message.get("id") else {
            continue;
        };
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let result = match method {
            "initialize" => json!({"capabilities": {"documentSymbolProvider": true}}),
            "textDocument/documentSymbol" => json!([{
                "name": "FixtureSymbol",
                "kind": 12,
                "range": {
                    "start": {"line": 0, "character": 0},
                    "end": {"line": 0, "character": 1}
                },
                "selectionRange": {
                    "start": {"line": 0, "character": 0},
                    "end": {"line": 0, "character": 1}
                }
            }]),
            _ => Value::Null,
        };
        write_lsp_message(
            &mut stdout,
            &json!({"jsonrpc": "2.0", "id": id, "result": result}),
        );
    }
}

fn serve_plugin() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let value: Value = serde_json::from_str(input.trim()).unwrap();
    println!("{}", json!({"received": value}));
}

fn serve_codex() {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut stdout = io::stdout().lock();
    while let Some(Ok(line)) = lines.next() {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        if method == "initialized" {
            continue;
        }
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let result = match method {
            "initialize" => json!({"userAgent": "kernex-fixture"}),
            "account/read" => json!({
                "account": {"type": "chatgpt", "email": "fixture@example.com", "planType": "plus"},
                "requiresOpenaiAuth": true
            }),
            "account/rateLimits/read" => json!({
                "rateLimits": {
                    "limitId": "codex",
                    "planType": "plus",
                    "primary": {"usedPercent": 25, "windowDurationMins": 300, "resetsAt": 1800000000}
                },
                "rateLimitsByLimitId": {},
                "rateLimitResetCredits": {"availableCount": 2}
            }),
            "model/list" => json!({
                "data": [{
                    "id": "gpt-fixture",
                    "displayName": "GPT Fixture",
                    "description": "Subscription fixture model",
                    "isDefault": true
                }],
                "nextCursor": null
            }),
            "account/login/start" => {
                write_json_line(
                    &mut stdout,
                    &json!({"id": id, "result": {
                        "type": "chatgpt",
                        "loginId": "login-fixture",
                        "authUrl": "https://example.com/login"
                    }}),
                );
                write_json_line(
                    &mut stdout,
                    &json!({"method": "account/login/completed", "params": {
                        "loginId": "login-fixture", "success": true, "error": null
                    }}),
                );
                continue;
            }
            "account/logout" => json!({}),
            "thread/start" | "thread/resume" => json!({
                "thread": {"id": "thread-fixture", "sessionId": "thread-fixture"},
                "model": "gpt-fixture",
                "modelProvider": "openai",
                "cwd": "/workspace",
                "approvalPolicy": "on-request",
                "approvalsReviewer": "user",
                "sandbox": {"type": "workspaceWrite"}
            }),
            "turn/start" => {
                write_json_line(
                    &mut stdout,
                    &json!({"id": id, "result": {"turn": {
                        "id": "turn-fixture", "status": "inProgress", "items": []
                    }}}),
                );
                write_json_line(
                    &mut stdout,
                    &json!({"method": "item/started", "params": {
                        "threadId": "thread-fixture", "turnId": "turn-fixture", "item": {
                            "id": "command-fixture", "type": "commandExecution", "command": "cargo test",
                            "commandActions": [], "cwd": "/workspace", "status": "inProgress"
                        }
                    }}),
                );
                write_json_line(
                    &mut stdout,
                    &json!({"id": 900, "method": "item/commandExecution/requestApproval", "params": {
                        "threadId": "thread-fixture", "turnId": "turn-fixture",
                        "itemId": "command-fixture", "command": "cargo test", "cwd": "/workspace",
                        "reason": "run fixture tests", "startedAtMs": 1
                    }}),
                );
                let approval = lines
                    .next()
                    .and_then(Result::ok)
                    .and_then(|line| serde_json::from_str::<Value>(&line).ok())
                    .expect("fixture approval response");
                assert_eq!(approval["id"], 900);
                assert_eq!(approval["result"]["decision"], "acceptForSession");
                write_json_line(
                    &mut stdout,
                    &json!({"method": "item/completed", "params": {
                        "threadId": "thread-fixture", "turnId": "turn-fixture", "completedAtMs": 2,
                        "item": {"id": "command-fixture", "type": "commandExecution", "command": "cargo test",
                            "commandActions": [], "cwd": "/workspace", "status": "completed",
                            "aggregatedOutput": "tests passed", "exitCode": 0}
                    }}),
                );
                write_json_line(
                    &mut stdout,
                    &json!({"method": "item/agentMessage/delta", "params": {
                        "threadId": "thread-fixture", "turnId": "turn-fixture",
                        "itemId": "message-fixture", "delta": "Fixture complete"
                    }}),
                );
                write_json_line(
                    &mut stdout,
                    &json!({"method": "item/completed", "params": {
                        "threadId": "thread-fixture", "turnId": "turn-fixture", "completedAtMs": 3,
                        "item": {"id": "message-fixture", "type": "agentMessage",
                            "phase": "final_answer", "text": "Fixture complete"}
                    }}),
                );
                write_json_line(
                    &mut stdout,
                    &json!({"method": "thread/tokenUsage/updated", "params": {
                        "threadId": "thread-fixture", "turnId": "turn-fixture",
                        "tokenUsage": {"last": {"inputTokens": 7, "outputTokens": 3}, "total": {}}
                    }}),
                );
                write_json_line(
                    &mut stdout,
                    &json!({"method": "turn/completed", "params": {
                        "threadId": "thread-fixture", "turn": {
                            "id": "turn-fixture", "status": "completed", "items": []
                        }
                    }}),
                );
                continue;
            }
            "turn/interrupt" => json!({}),
            _ => {
                write_json_line(
                    &mut stdout,
                    &json!({"id": id, "error": {"code": -32601, "message": "unknown fixture method"}}),
                );
                continue;
            }
        };
        write_json_line(&mut stdout, &json!({"id": id, "result": result}));
    }
}

fn write_json_line(writer: &mut impl Write, value: &Value) {
    writeln!(writer, "{value}").unwrap();
    writer.flush().unwrap();
}

fn read_lsp_message(reader: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 {
            return None;
        }
        if header == "\r\n" || header == "\n" {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length:") {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length?];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn write_lsp_message(writer: &mut impl Write, value: &Value) {
    let body = value.to_string();
    write!(writer, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    writer.flush().unwrap();
}
