use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::thread;

type Log = Arc<Mutex<Vec<(String, String, Value)>>>;

struct Mock {
    url: String,
    log: Log,
}

fn handle(stream: TcpStream, projects: &Mutex<Vec<Value>>, log: &Log) {
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
        .any(|kv| kv.starts_with("%24skip=") && kv != "%24skip=0")
        || query
            .split('&')
            .any(|kv| kv.starts_with("$skip=") && kv != "$skip=0");

    let (status, resp) = match (method.as_str(), path) {
        ("GET", _) if later_page => (200, json!([])),
        ("GET", "/api/admin/projects") => (200, Value::Array(projects.lock().unwrap().clone())),
        ("GET", "/api/users") => (200, json!([{"id": "1-5", "login": "alice"}])),
        ("POST", p) if p.starts_with("/api/admin/projects/") => {
            let id = &p["/api/admin/projects/".len()..];
            let mut ps = projects.lock().unwrap();
            match ps.iter_mut().find(|x| x["id"] == id) {
                Some(proj) => {
                    for (k, v) in body.as_object().unwrap() {
                        proj[k] = v.clone();
                    }
                    (200, proj.clone())
                }
                None => (404, json!({"error": "Not Found"})),
            }
        }
        _ => (404, json!({"error": "Not Found"})),
    };
    let data = resp.to_string();
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
    let projects = Arc::new(Mutex::new(vec![
        json!({"id": "0-7", "shortName": "DEMO", "name": "Demo", "archived": false}),
    ]));
    let log: Log = Arc::default();
    let l = log.clone();
    thread::spawn(move || {
        for s in listener.incoming().flatten() {
            handle(s, &projects, &l);
        }
    });
    Mock { url, log }
}

fn yt(m: &Mock, args: &[&str]) -> Output {
    let home = std::env::temp_dir().join("yt-project-edit-test-home");
    Command::new(env!("CARGO_BIN_EXE_yt"))
        .args(args)
        .env("YOUTRACK_URL", &m.url)
        .env("YOUTRACK_API_TOKEN", "t")
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

#[test]
fn rename_is_reflected_in_project_ls() {
    let m = mock();
    let o = yt(
        &m,
        &["write", "project", "edit", "demo", "--name", "Demo App"],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stdout(&o), "DEMO  Demo App\n");
    let ls = yt(&m, &["read", "project", "ls"]);
    assert!(ls.status.success(), "{}", stderr(&ls));
    assert_eq!(stdout(&ls), "DEMO  Demo App\n");
}

#[test]
fn edit_posts_all_fields_to_the_project_id() {
    let m = mock();
    let o = yt(
        &m,
        &[
            "write",
            "project",
            "edit",
            "Demo",
            "--description",
            "D",
            "--leader",
            "alice",
            "--archived",
            "true",
        ],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let log = m.log.lock().unwrap();
    let post = log.iter().find(|(meth, _, _)| meth == "POST").unwrap();
    assert_eq!(post.1, "/api/admin/projects/0-7");
    assert_eq!(
        post.2,
        json!({"description": "D", "leader": {"id": "1-5"}, "archived": true})
    );
    drop(log);
    assert_eq!(stdout(&yt(&m, &["read", "project", "ls"])), "");
    assert_eq!(
        stdout(&yt(&m, &["read", "project", "ls", "--all"])),
        "DEMO  Demo\n"
    );
}

#[test]
fn unknown_project_is_a_clear_error() {
    let m = mock();
    let o = yt(&m, &["write", "project", "edit", "NOPE", "--name", "X"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(stderr(&o).trim(), "error: project not found: NOPE");
    assert!(m
        .log
        .lock()
        .unwrap()
        .iter()
        .all(|(meth, _, _)| meth == "GET"));
}

#[test]
fn no_flag_is_a_no_op() {
    let m = mock();
    let o = yt(&m, &["write", "project", "edit", "DEMO"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stdout(&o), "");
    assert!(stderr(&o).contains("nothing to change"));
    assert!(m.log.lock().unwrap().is_empty());
}
