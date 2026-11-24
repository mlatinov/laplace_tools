//! The server over stdio, as an editor runs it: the real binary, framed
//! JSON-RPC in and out. Guards the startup bug where the server answered
//! `-32700 Parse error` before any input instead of waiting for it (fixed
//! by `SkipLeadingWhitespace` in `main.rs`).

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Duration;

use serde_json::{json, Value};

fn server() -> Child {
    Command::new(env!("CARGO_BIN_EXE_laplace-lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start laplace-lsp")
}

fn send(child: &mut Child, msg: Value) {
    let body = msg.to_string();
    let stdin = child.stdin.as_mut().unwrap();
    write!(stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    stdin.flush().unwrap();
}

/// One framed message from the server.
fn recv(out: &mut BufReader<ChildStdout>) -> Value {
    let mut len = None;
    loop {
        let mut line = String::new();
        assert!(
            out.read_line(&mut line).unwrap() > 0,
            "server closed stdout"
        );
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(n) = line.strip_prefix("Content-Length: ") {
            len = Some(n.parse::<usize>().unwrap());
        }
    }
    let mut body = vec![0; len.expect("Content-Length header")];
    out.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// The next message that is a response (skipping notifications such as
/// `window/logMessage`).
fn response(out: &mut BufReader<ChildStdout>, id: u64) -> Value {
    loop {
        let m = recv(out);
        if m.get("id") == Some(&json!(id)) {
            return m;
        }
        assert!(
            m.get("method").is_some(),
            "expected a notification or response {id}, got {m}"
        );
    }
}

#[test]
fn initialize_initialized_shutdown_exit() {
    let mut child = server();
    let mut out = BufReader::new(child.stdout.take().unwrap());

    send(
        &mut child,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"processId": null, "rootUri": null, "capabilities": {}}}),
    );
    let init = response(&mut out, 1);
    assert!(init.get("error").is_none(), "{init}");
    assert_eq!(init["result"]["serverInfo"]["name"], "laplace-lsp");
    assert!(init["result"]["capabilities"]["textDocumentSync"].is_number());

    send(
        &mut child,
        json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
    );
    send(
        &mut child,
        json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown"}),
    );
    let shut = response(&mut out, 2);
    assert_eq!(shut["result"], Value::Null, "{shut}");

    send(&mut child, json!({"jsonrpc": "2.0", "method": "exit"}));
    drop(child.stdin.take());
    let status = child.wait().unwrap();
    assert!(status.success(), "exit status {status}");

    // What is left must be well-framed notifications (the "initialized"
    // log message is sent asynchronously and may come after the shutdown
    // response): no responses, no unframed text.
    loop {
        let mut peek = String::new();
        if out.read_line(&mut peek).unwrap() == 0 {
            break;
        }
        let len: usize = peek
            .trim_end()
            .strip_prefix("Content-Length: ")
            .unwrap_or_else(|| panic!("stray output after exit: {peek:?}"))
            .parse()
            .unwrap();
        let mut blank = String::new();
        out.read_line(&mut blank).unwrap();
        assert_eq!(blank, "\r\n", "stray output after exit");
        let mut body = vec![0; len];
        out.read_exact(&mut body).unwrap();
        let m: Value = serde_json::from_slice(&body).unwrap();
        assert!(m.get("method").is_some() && m.get("id").is_none(), "{m}");
    }
    let mut err = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    assert_eq!(err, "", "stray stderr");
}

#[test]
fn waits_for_input_instead_of_answering_nothing() {
    let mut child = server();
    // A stray newline first (a terminal user pressing Enter) must be skipped
    // too, not answered with a parse error.
    child.stdin.as_mut().unwrap().write_all(b"\r\n").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        child.try_wait().unwrap().is_none(),
        "the server exited without input"
    );
    // Closing stdin ends it cleanly, having written nothing.
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
}

/// The next `textDocument/publishDiagnostics` for `uri`.
fn diagnostics_for(out: &mut BufReader<ChildStdout>, uri: &str) -> Vec<Value> {
    loop {
        let m = recv(out);
        if m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri {
            return m["params"]["diagnostics"].as_array().unwrap().clone();
        }
    }
}

#[test]
fn a_lapkernel_gets_diagnostics_as_it_is_opened_edited_and_closed() {
    let mut child = server();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    send(
        &mut child,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"processId": null, "rootUri": null, "capabilities": {}}}),
    );
    response(&mut out, 1);
    send(
        &mut child,
        json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
    );

    let uri = "file:///nowhere/k.lapkernel";
    let good = "kernel k {\n  step {\n    move(q);\n  }\n}\n";
    let bad = "kernel k {\n  step {\n    move(nope);\n  }\n}\n";
    send(
        &mut child,
        json!({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {"textDocument":
               {"uri": uri, "languageId": "lapkernel", "version": 1, "text": bad}}}),
    );
    let ds = diagnostics_for(&mut out, uri);
    assert_eq!(ds.len(), 1, "{ds:?}");
    assert_eq!(ds[0]["message"], "unknown name `nope`");
    assert_eq!(ds[0]["source"], "laplace sampler");
    assert_eq!(ds[0]["severity"], 1);
    assert_eq!(
        ds[0]["range"],
        json!({"start": {"line": 2, "character": 9}, "end": {"line": 2, "character": 13}})
    );

    send(
        &mut child,
        json!({"jsonrpc": "2.0", "method": "textDocument/didChange", "params": {
               "textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": good}]}}),
    );
    assert_eq!(diagnostics_for(&mut out, uri), Vec::<Value>::new());

    send(
        &mut child,
        json!({"jsonrpc": "2.0", "method": "textDocument/didChange", "params": {
               "textDocument": {"uri": uri, "version": 3}, "contentChanges": [{"text": bad}]}}),
    );
    assert_eq!(diagnostics_for(&mut out, uri).len(), 1);
    send(
        &mut child,
        json!({"jsonrpc": "2.0", "method": "textDocument/didClose", "params": {
               "textDocument": {"uri": uri}}}),
    );
    assert_eq!(diagnostics_for(&mut out, uri), Vec::<Value>::new());

    // No .laplace features answer for a kernel: no semantic tokens.
    send(
        &mut child,
        json!({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {"textDocument":
               {"uri": uri, "languageId": "lapkernel", "version": 4, "text": good}}}),
    );
    send(
        &mut child,
        json!({"jsonrpc": "2.0", "id": 2, "method": "textDocument/semanticTokens/full",
               "params": {"textDocument": {"uri": uri}}}),
    );
    assert_eq!(response(&mut out, 2)["result"], Value::Null);

    send(
        &mut child,
        json!({"jsonrpc": "2.0", "id": 3, "method": "shutdown"}),
    );
    response(&mut out, 3);
    send(&mut child, json!({"jsonrpc": "2.0", "method": "exit"}));
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
}
