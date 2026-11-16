//! `mobicat mcp`: a Model Context Protocol server over stdio.
//!
//! Each tool runs mobicat itself with the matching flags (`--last`, `--for`,
//! `-q`), so the tools behave exactly like the command line described in
//! docs/agents.md. Messages are newline-delimited JSON-RPC 2.0.

use std::io::{self, BufRead, Write};
use std::process::Command;

use anyhow::Result;
use serde_json::{Value, json};

/// Newest protocol revision this server was written against. A client that
/// asks for another one gets its own version echoed back; the tools only use
/// the basic `tools/list` and `tools/call` shapes, which have not changed.
const PROTOCOL: &str = "2025-06-18";

/// Output kept per call. Logs are cut from the start, so the newest lines stay.
const MAX_OUTPUT: usize = 100_000;

/// Longest `follow`, so a call cannot hang the agent for long.
const MAX_FOLLOW_SECS: u64 = 300;

pub fn serve() -> Result<()> {
    let stdin = io::stdin();
    let mut out = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&msg),
            Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(r) = reply {
            serde_json::to_writer(&mut out, &r)?;
            out.write_all(b"\n")?;
            out.flush()?;
        }
    }
    Ok(())
}

/// A reply, or `None` for notifications.
fn handle(msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let id = id?; // notifications (initialized, cancelled) need no reply
    Some(match method {
        "initialize" => {
            let version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL);
            ok(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "mobicat", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": INSTRUCTIONS,
                }),
            )
        }
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match args_for(name, &args) {
                Ok(argv) => ok(id, run(&argv)),
                Err(m) => ok(id, text_result(&m, true)),
            }
        }
        _ => error(id, -32601, &format!("method not found: {method}")),
    })
}

const INSTRUCTIONS: &str = "Read an Android or iOS app's logs and crashes. \
Call `crashes` after the app crashed, `logs` for what happened recently, and \
`follow` to capture while the user tries something (or with launch=true to \
capture a startup). The app is a package name or bundle ID, e.g. com.example.app.";

fn tools() -> Value {
    let app = json!({ "type": "string", "description": "Package name (Android) or bundle ID (iOS). Optional when .mobicat.toml sets `app`." });
    let query = json!({ "type": "string", "description": "Filter query, e.g. `level:warn`, `tag:OkHttp`, `-tag:Chatty`, `is:crash`." });
    let device = json!({ "type": "string", "description": "Only this device: Android serial, simulator UDID or iPhone UDID (see `devices`)." });
    let json_out = json!({ "type": "boolean", "description": "NDJSON with every field instead of one line per entry. Default false." });
    json!([
        {
            "name": "devices",
            "description": "List connected Android devices, iPhones and booted iOS simulators.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "logs",
            "description": "The app's own logs from the last while, from every connected device, oldest first. iPhones only have new lines; use `follow` for them.",
            "inputSchema": { "type": "object", "properties": {
                "app": app, "query": query, "device": device, "json": json_out,
                "last": { "type": "string", "description": "How far back: 30s, 5m, 1h. Default 5m." },
            } },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "crashes",
            "description": "Only crashes, each with its whole stack trace (and the crash report on iOS simulators).",
            "inputSchema": { "type": "object", "properties": {
                "app": app, "device": device, "json": json_out,
                "last": { "type": "string", "description": "How far back. Default 30m." },
            } },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "follow",
            "description": "Capture the app's logs for a while, then return them. With launch=true the app is restarted first, so its startup is captured (on iOS simulators also print() output).",
            "inputSchema": { "type": "object", "properties": {
                "app": app, "query": query, "device": device, "json": json_out,
                "for": { "type": "string", "description": "How long: 10s, 30s, 2m. Default 20s, at most 5m." },
                "launch": { "type": "boolean", "description": "Restart the app first. Default false." },
            } },
        },
    ])
}

fn s<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
}

fn b(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// The command line for a tool call.
fn args_for(name: &str, args: &Value) -> Result<Vec<String>, String> {
    let mut argv: Vec<String> = Vec::new();
    if name == "devices" {
        return Ok(vec!["devices".into()]);
    }
    if !matches!(name, "logs" | "crashes" | "follow") {
        return Err(format!("unknown tool `{name}`"));
    }
    if let Some(app) = s(args, "app") {
        argv.push(app.into());
    }
    if let Some(d) = s(args, "device") {
        argv.extend(["--device".into(), d.into()]);
    }
    match name {
        "logs" => argv.extend(["--last".into(), s(args, "last").unwrap_or("5m").into()]),
        "crashes" => {
            argv.extend(["--last".into(), s(args, "last").unwrap_or("30m").into()]);
            argv.extend(["--query".into(), "is:crash".into()]);
        }
        _ => {
            let span = s(args, "for").unwrap_or("20s");
            let secs = mobicat_core::query::parse_duration(span)
                .ok_or_else(|| format!("`for` should look like 30s or 2m, not `{span}`"))?
                .as_secs();
            if secs > MAX_FOLLOW_SECS {
                return Err(format!("`for` can be at most 5m, not `{span}`"));
            }
            argv.extend(["--for".into(), span.into()]);
            if b(args, "launch") {
                argv.push("--launch".into());
            }
        }
    }
    if name != "crashes"
        && let Some(q) = s(args, "query")
    {
        argv.extend(["--query".into(), q.into()]);
    }
    if b(args, "json") {
        argv.push("--json".into());
    } else {
        argv.extend(["--color".into(), "never".into()]);
    }
    Ok(argv)
}

fn run(argv: &[String]) -> Value {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return text_result(&format!("cannot find the mobicat executable: {e}"), true),
    };
    let out = match Command::new(exe).args(argv).env("NO_COLOR", "1").output() {
        Ok(o) => o,
        Err(e) => return text_result(&format!("could not run mobicat: {e}"), true),
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut text = String::new();
    if !stderr.trim().is_empty() {
        text.push_str(stderr.trim_end());
        text.push('\n');
    }
    text.push_str(&stdout);
    if text.trim().is_empty() {
        text = "No matching logs.".into();
    }
    text_result(&keep_tail(&text, MAX_OUTPUT), !out.status.success())
}

/// The last `max` bytes, cut at a line start, with a note when cut.
fn keep_tail(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let start = text[start..].find('\n').map_or(start, |i| start + i + 1);
    format!(
        "[{} earlier bytes left out; narrow it with `query` or a shorter `last`]\n{}",
        start,
        &text[start..]
    )
}

fn text_result(text: &str, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_tools_to_flags() {
        let a = |n: &str, v: Value| args_for(n, &v).unwrap().join(" ");
        assert_eq!(a("devices", json!({})), "devices");
        assert_eq!(
            a(
                "logs",
                json!({"app": "com.example.app", "query": "level:warn"})
            ),
            "com.example.app --last 5m --query level:warn --color never"
        );
        assert_eq!(
            a("crashes", json!({"app": "x", "last": "1h", "json": true})),
            "x --last 1h --query is:crash --json"
        );
        assert_eq!(
            a(
                "follow",
                json!({"app": "x", "for": "30s", "launch": true, "device": "emulator-5554"})
            ),
            "x --device emulator-5554 --for 30s --launch --color never"
        );
        assert!(args_for("follow", &json!({"for": "10m"})).is_err());
        assert!(args_for("follow", &json!({"for": "soon"})).is_err());
        assert!(args_for("rm", &json!({})).is_err());
    }

    #[test]
    fn answers_the_handshake() {
        let r = handle(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}})).unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "mobicat");
        assert!(handle(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})).is_none());
        let r = handle(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).unwrap();
        let names: Vec<_> = r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["devices", "logs", "crashes", "follow"]);
        let r = handle(&json!({"jsonrpc":"2.0","id":3,"method":"nope"})).unwrap();
        assert_eq!(r["error"]["code"], -32601);
    }

    #[test]
    fn keeps_the_newest_output() {
        let text = "old line\nmiddle line\nnew line\n";
        let kept = keep_tail(text, 12);
        assert!(kept.ends_with("new line\n"), "{kept}");
        assert!(kept.starts_with("[21 earlier bytes left out"), "{kept}");
        assert_eq!(keep_tail("short", 100), "short");
    }
}
