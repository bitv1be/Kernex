use std::io::{self, BufRead, Read, Write};

use serde_json::{Value, json};

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("mcp") => serve_mcp(),
        Some("lsp") => serve_lsp(),
        Some("plugin") => serve_plugin(),
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
