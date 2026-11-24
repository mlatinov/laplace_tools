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

    let mut rest = String::new();
    out.read_to_string(&mut rest).unwrap();
    assert_eq!(rest, "", "stray output after exit");
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
