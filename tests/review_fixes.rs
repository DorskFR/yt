use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::thread;

type Log = Arc<Mutex<Vec<(String, String, Value)>>>;

struct Mock {
    url: String,
    log: Log,
}

const ESC_SUMMARY: &str = "safe\u{1b}[2K\u{1b}[1Aevil";

fn route(method: &str, path: &str, body: &Value) -> (u16, String) {
    let j = |v: Value| v.to_string();
    match (method, path) {
        ("GET", "/api/issues") => (
            200,
            j(json!([
                {"idReadable": "DEMO-1", "summary": "one", "customFields": []},
                {"idReadable": "DEMO-2", "summary": "two", "customFields": []},
            ])),
        ),
        ("GET", "/api/issues/DEMO-1") => (
            200,
            j(json!({
                "idReadable": "DEMO-1",
                "summary": ESC_SUMMARY,
                "description": "desc \u{1b}]52;c;aGk=\u{7} end",
                "reporter": {"login": "r\u{9b}31m"},
                "customFields": [{"name": "State", "value": {"name": "Open\u{1b}[8m"}}],
                "tags": [{"name": "t\u{202e}ag"}],
            })),
        ),
        ("GET", "/api/issues/DEMO-1/comments") => (
            200,
            j(json!([{
                "id": "4-1",
                "author": {"login": "bob\u{1b}[0m"},
                "created": 0,
                "text": "hello\u{1b}[2K\u{1b}[1Aworld",
            }])),
        ),
        ("GET", "/api/issues/DEMO-1/attachments") => (
            200,
            j(json!([
                {"id": "8-1", "name": "Makefile", "size": 7, "url": "/api/files/8-1"},
                {"id": "8-2", "name": ".envrc", "size": 7, "url": "/api/files/8-2"},
                {"id": "8-3", "name": "../../escape.txt", "size": 7, "url": "/api/files/8-3"},
            ])),
        ),
        ("GET", p) if p.starts_with("/api/files/") => (200, format!("PAYLOAD{}", &p[11..])),
        ("GET", p) if p.ends_with("/activities") || p.ends_with("/links") => (200, j(json!([]))),
        ("GET", "/api/admin/projects") => (
            200,
            j(json!([{"id": "0-7", "shortName": "DEMO", "name": "Demo"}])),
        ),
        ("GET", "/api/admin/projects/0-7/customFields") => (
            200,
            j(json!([
                {"field": {"name": "State", "fieldType": {"id": "state[1]"}}},
                {"field": {"name": "Priority", "fieldType": {"id": "enum[1]"}}},
            ])),
        ),
        ("POST", "/api/issues") if body.get("customFields").is_some() => (
            400,
            j(json!({"error": "bad", "error_description": "Unknown value"})),
        ),
        ("POST", "/api/issues") => (200, j(json!({"idReadable": "DEMO-9"}))),
        ("POST", "/api/commands") => (200, j(json!({}))),
        _ => (404, j(json!({"error": "Not Found"}))),
    }
}

fn handle(stream: TcpStream, log: &Log) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let mut len = 0;
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).unwrap();
        if h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().unwrap();
            }
        }
    }
    let mut raw = vec![0; len];
    reader.read_exact(&mut raw).unwrap();
    let body: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    log.lock()
        .unwrap()
        .push((method.clone(), path.to_string(), body.clone()));
    let later_page = query
        .split('&')
        .any(|kv| (kv.starts_with("%24skip=") || kv.starts_with("$skip=")) && !kv.ends_with("=0"));
    let (status, data) = if method == "GET" && later_page && path != "/api/issues" {
        (200, "[]".to_string())
    } else {
        route(&method, path, &body)
    };
    let mut out = stream;
    write!(
        out,
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{data}",
        data.len()
    )
    .unwrap();
}

fn mock() -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::default();
    let l = log.clone();
    thread::spawn(move || {
        for s in listener.incoming().flatten() {
            handle(s, &l);
        }
    });
    Mock { url, log }
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("yt-review-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn yt(m: &Mock, args: &[&str]) -> Output {
    let home = std::env::temp_dir().join("yt-review-test-home");
    Command::new(env!("CARGO_BIN_EXE_yt"))
        .arg("--agent")
        .args(args)
        .env("YOUTRACK_URL", &m.url)
        .env("YOUTRACK_API_TOKEN", "t")
        .env("YT_NO_UPDATE_CHECK", "1")
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .output()
        .expect("run yt")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn assert_no_raw_controls(s: &str) {
    assert!(
        !s.chars()
            .any(|c| (c.is_control() && c != '\n' && c != '\t') || c == '\u{202e}'),
        "raw control characters in output: {s:?}"
    );
}

#[test]
fn show_and_comments_escape_terminal_controls() {
    let m = mock();
    let o = yt(&m, &["read", "issue", "show", "DEMO-1", "-c"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    assert_no_raw_controls(&out);
    assert!(out.contains("safe\\x1b[2K\\x1b[1Aevil"), "{out}");
    assert!(out.contains("desc \\x1b]52;c;aGk=\\x07 end"), "{out}");
    assert!(out.contains("hello\\x1b[2K\\x1b[1Aworld"), "{out}");
    assert!(out.contains("bob\\x1b[0m"), "{out}");
    assert!(out.contains("t\\u{202e}ag"), "{out}");

    let o = yt(&m, &["read", "issue", "comments", "DEMO-1"]);
    assert_no_raw_controls(&stdout(&o));
}

#[test]
fn json_output_keeps_the_raw_text() {
    let m = mock();
    let o = yt(&m, &["read", "--json", "issue", "show", "DEMO-1"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["summary"], ESC_SUMMARY);
}

#[test]
fn attachments_download_never_overwrites() {
    let m = mock();
    let dir = scratch("attachments");
    let d = dir.to_str().unwrap();
    std::fs::write(dir.join("Makefile"), "original").unwrap();

    let o = yt(&m, &["read", "issue", "attachments", "DEMO-1", "-o", d]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(
        std::fs::read_to_string(dir.join("Makefile")).unwrap(),
        "original"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("Makefile-8-1")).unwrap(),
        "PAYLOAD8-1"
    );
    assert!(!dir.join(".envrc").exists());
    assert_eq!(
        std::fs::read_to_string(dir.join("_.envrc")).unwrap(),
        "PAYLOAD8-2"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("escape.txt")).unwrap(),
        "PAYLOAD8-3"
    );

    let o = yt(&m, &["read", "issue", "attachments", "DEMO-1", "-o", d]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(
        std::fs::read_to_string(dir.join("Makefile")).unwrap(),
        "original"
    );
    assert!(stderr(&o).contains("skipped Makefile"), "{}", stderr(&o));
    assert_eq!(
        std::fs::read_to_string(dir.join("_-8-2.envrc")).unwrap(),
        "PAYLOAD8-2"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn merged_pr_empty_page_keeps_the_next_page_hint() {
    let m = mock();
    let o = yt(
        &m,
        &[
            "read",
            "issue",
            "ls",
            "project: DEMO",
            "-n",
            "2",
            "--merged-pr",
        ],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "no matches in this page (2 fetched)");
    assert!(
        stderr(&o).contains("# limit 2 reached; next page: --skip 2"),
        "{}",
        stderr(&o)
    );
}

fn create_requests(m: &Mock) -> Vec<(String, Value)> {
    m.log
        .lock()
        .unwrap()
        .iter()
        .filter(|(method, p, _)| method == "POST" && (p == "/api/issues" || p == "/api/commands"))
        .map(|(_, p, b)| (p.clone(), b.clone()))
        .collect()
}

#[test]
fn braced_field_value_uses_the_command_fallback() {
    let m = mock();
    let o = yt(
        &m,
        &[
            "write",
            "issue",
            "new",
            "DEMO",
            "x",
            "-f",
            "State {In Progress}",
        ],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "DEMO-9");
    let reqs = create_requests(&m);
    assert_eq!(reqs.len(), 2, "{reqs:?}");
    assert!(reqs[0].1.get("customFields").is_none());
    assert_eq!(reqs[1].0, "/api/commands");
    assert_eq!(reqs[1].1["query"], "State {In Progress}");
}

#[test]
fn rejected_native_fields_retry_without_them() {
    let m = mock();
    let o = yt(
        &m,
        &["write", "issue", "new", "DEMO", "x", "-f", "Priority Bogus"],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "DEMO-9");
    let reqs = create_requests(&m);
    assert_eq!(reqs.len(), 3, "{reqs:?}");
    assert!(reqs[0].1.get("customFields").is_some());
    assert!(reqs[1].1.get("customFields").is_none());
    assert_eq!(reqs[2].1["query"], "Priority Bogus");
}
