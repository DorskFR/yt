use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use std::io::Read;

const LIST_FIELDS: &str = "idReadable,summary,customFields(name,value(name,login,text))";
const ISSUE_FIELDS: &str = "idReadable,summary,description,created,updated,reporter(login),customFields(name,value(name,login,text))";
const COMMENT_FIELDS: &str = "created,text,author(login)";

#[derive(Parser)]
#[command(name = "yt", version, about = "Token-frugal YouTrack CLI (env: YOUTRACK_URL, YOUTRACK_API_TOKEN)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Search issues, one line each: ID  STATE  PRIO  SUMMARY
    Ls {
        /// YouTrack query, e.g. "project: DEMO #Unresolved sort by: updated desc"
        query: String,
        /// Max results
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
        /// Include descriptions
        #[arg(long)]
        full: bool,
    },
    /// Show one issue
    Show {
        id: String,
        /// Include comments
        #[arg(short, long)]
        comments: bool,
    },
    /// Create an issue; prints the new ID only
    New {
        /// Project short name or name, e.g. DEMO
        project: String,
        summary: String,
        /// Description ("-" reads stdin)
        #[arg(short, long)]
        desc: Option<String>,
        /// Field assignment in command syntax, repeatable: -f "Priority Critical" -f "Type Bug"
        #[arg(short, long)]
        field: Vec<String>,
    },
    /// Add a comment (text arg, or stdin if omitted)
    Comment { id: String, text: Option<String> },
    /// List an issue's comments
    Comments { id: String },
    /// Apply a YouTrack command to issues, e.g. yt cmd "State Fixed assignee me" DEMO-1 DEMO-2
    Cmd {
        command: String,
        #[arg(required = true)]
        ids: Vec<String>,
        /// Comment to add alongside the command
        #[arg(short = 'm', long)]
        comment: Option<String>,
    },
    /// List projects
    Projects,
    /// Show a project's custom fields and allowed values
    Fields { project: String },
    /// Show the authenticated user
    Me,
    /// Search users by name/login
    Users { query: String },
    /// Print query syntax cheat sheet
    QueryHelp,
    /// Save credentials to ~/.config/yt/config.json (env vars still take precedence)
    Auth {
        /// YouTrack base URL, e.g. https://youtrack.example.com
        url: String,
        /// Permanent API token ("-" reads stdin)
        token: String,
    },
}

fn config_path() -> std::path::PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .unwrap_or_default()
        .join("yt/config.json")
}

struct Client {
    base: String,
    token: String,
}

impl Client {
    fn from_env() -> Result<Self> {
        let mut url = std::env::var("YOUTRACK_URL").ok();
        let mut token = std::env::var("YOUTRACK_API_TOKEN").ok();
        if url.is_none() || token.is_none() {
            if let Ok(s) = std::fs::read_to_string(config_path()) {
                let cfg: Value = serde_json::from_str(&s)
                    .with_context(|| format!("invalid JSON in {}", config_path().display()))?;
                url = url.or_else(|| cfg["url"].as_str().map(String::from));
                token = token.or_else(|| cfg["token"].as_str().map(String::from));
            }
        }
        let url = url.context("no YouTrack URL: set YOUTRACK_URL or run `yt auth URL TOKEN`")?;
        let token = token.context("no API token: set YOUTRACK_API_TOKEN or run `yt auth URL TOKEN`")?;
        let url = url.trim_end_matches('/');
        let base = if url.ends_with("/api") { url.to_string() } else { format!("{url}/api") };
        Ok(Self { base, token })
    }

    fn req(&self, method: &str, path: &str, params: &[(&str, &str)]) -> ureq::Request {
        let mut r = ureq::request(method, &format!("{}/{}", self.base, path))
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Accept", "application/json");
        for (k, v) in params {
            r = r.query(k, v);
        }
        r
    }

    fn get(&self, path: &str, params: &[(&str, &str)]) -> Result<Value> {
        read(self.req("GET", path, params).call())
    }

    fn post(&self, path: &str, params: &[(&str, &str)], body: Value) -> Result<Value> {
        read(self.req("POST", path, params).send_json(body))
    }
}

fn read(res: Result<ureq::Response, ureq::Error>) -> Result<Value> {
    match res {
        Ok(r) => {
            let s = r.into_string()?;
            Ok(if s.is_empty() { Value::Null } else { serde_json::from_str(&s)? })
        }
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| {
                    let d = v["error_description"].as_str().or(v["error"].as_str())?;
                    Some(d.to_string())
                })
                .unwrap_or(body);
            bail!("HTTP {code}: {msg}")
        }
        Err(e) => bail!(e.to_string()),
    }
}

/// Epoch millis -> YYYY-MM-DD (civil-from-days, Hinnant).
fn date(v: &Value) -> String {
    let Some(ms) = v.as_i64() else { return "-".into() };
    let z = ms / 86_400_000 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Render a custom field value compactly; None when empty.
fn cf_value(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::Array(a) => {
            let parts: Vec<_> = a.iter().filter_map(cf_value).collect();
            if parts.is_empty() { None } else { Some(parts.join(",")) }
        }
        Value::Object(o) => o
            .get("login")
            .or(o.get("name"))
            .or(o.get("text"))
            .and_then(Value::as_str)
            .map(String::from),
        // date custom fields come back as epoch millis
        Value::Number(n) if n.as_i64().is_some_and(|x| x > 100_000_000_000) => Some(date(v)),
        other => Some(other.to_string()),
    }
}

fn cf_get(issue: &Value, name: &str) -> Option<String> {
    issue["customFields"]
        .as_array()?
        .iter()
        .find(|f| f["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name)))
        .and_then(|f| cf_value(&f["value"]))
}

fn stdin_text() -> Result<String> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s)?;
    Ok(s.trim_end().to_string())
}

fn resolve_project(c: &Client, key: &str) -> Result<(String, String)> {
    let projects = c.get("admin/projects", &[("fields", "id,shortName,name"), ("$top", "500")])?;
    projects
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| {
            p["shortName"].as_str().is_some_and(|s| s.eq_ignore_ascii_case(key))
                || p["name"].as_str().is_some_and(|s| s.eq_ignore_ascii_case(key))
        })
        .map(|p| (p["id"].as_str().unwrap_or_default().to_string(), p["shortName"].as_str().unwrap_or(key).to_string()))
        .with_context(|| format!("project not found: {key}"))
}

fn issue_ref(id: &str) -> Value {
    // internal ids look like "2-123"; anything else is treated as readable (DEMO-1)
    let internal = id.split_once('-').is_some_and(|(a, b)| {
        !a.is_empty() && !b.is_empty() && a.chars().chain(b.chars()).all(|c| c.is_ascii_digit())
    });
    if internal { json!({"id": id}) } else { json!({"idReadable": id}) }
}

fn print_comments(c: &Client, id: &str) -> Result<()> {
    let comments = c.get(&format!("issues/{id}/comments"), &[("fields", COMMENT_FIELDS)])?;
    let list = comments.as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        println!("no comments");
    }
    for cm in &list {
        println!("[{} {}] {}", date(&cm["created"]), cm["author"]["login"].as_str().unwrap_or("?"), cm["text"].as_str().unwrap_or("").trim_end());
    }
    Ok(())
}

const QUERY_HELP: &str = "YouTrack query syntax:
  project: DEMO              #Unresolved | #Resolved | #me (assigned to me)
  State: Open               State: -Done (negate)   State: {In Progress} (multiword -> braces)
  Priority: Critical        assignee: me|login      reporter: login    type: Bug
  created: today            updated: {This week}    created: 2026-06-01 .. 2026-06-11
  summary: word             bare words = full-text search
  sort by: updated desc     sort by: priority asc
Terms combine with AND by default; use 'or' explicitly.
Examples:
  yt ls \"project: DEMO #Unresolved sort by: updated desc\"
  yt ls \"project: DEMO State: -Done assignee: me\"
  yt cmd \"State {In Progress} assignee me\" DEMO-12";

fn run() -> Result<()> {
    let cli = Cli::parse();
    if let Cmd::QueryHelp = cli.cmd {
        println!("{QUERY_HELP}");
        return Ok(());
    }
    if let Cmd::Auth { url, token } = &cli.cmd {
        let token = if token == "-" { stdin_text()? } else { token.clone() };
        let path = config_path();
        std::fs::create_dir_all(path.parent().context("no config dir")?)?;
        std::fs::write(&path, serde_json::to_string_pretty(&json!({"url": url, "token": token}))?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        println!("saved {}", path.display());
        return Ok(());
    }
    let c = Client::from_env()?;

    match cli.cmd {
        Cmd::Ls { query, limit, full } => {
            let fields = if full { format!("{LIST_FIELDS},description") } else { LIST_FIELDS.into() };
            let issues = c.get("issues", &[("query", &query), ("fields", &fields), ("$top", &limit.to_string())])?;
            let list = issues.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                println!("no matches");
                return Ok(());
            }
            for i in &list {
                println!(
                    "{}  {}  {}  {}",
                    i["idReadable"].as_str().unwrap_or("?"),
                    cf_get(i, "State").as_deref().unwrap_or("-"),
                    cf_get(i, "Priority").as_deref().unwrap_or("-"),
                    i["summary"].as_str().unwrap_or("")
                );
                if full {
                    if let Some(d) = i["description"].as_str().filter(|d| !d.is_empty()) {
                        for line in d.trim_end().lines() {
                            println!("  {line}");
                        }
                        println!();
                    }
                }
            }
            if list.len() == limit {
                println!("# limit {limit} reached; refine query or raise -n");
            }
        }
        Cmd::Show { id, comments } => {
            let i = c.get(&format!("issues/{id}"), &[("fields", ISSUE_FIELDS)])?;
            println!("{}  {}", i["idReadable"].as_str().unwrap_or(&id), i["summary"].as_str().unwrap_or(""));
            let mut meta: Vec<String> = i["customFields"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|f| Some(format!("{}:{}", f["name"].as_str()?, cf_value(&f["value"])?)))
                .collect();
            meta.push(format!("created:{}", date(&i["created"])));
            meta.push(format!("updated:{}", date(&i["updated"])));
            if let Some(r) = i["reporter"]["login"].as_str() {
                meta.push(format!("by:{r}"));
            }
            println!("{}", meta.join("  "));
            if let Some(d) = i["description"].as_str().filter(|d| !d.is_empty()) {
                println!("\n{}", d.trim_end());
            }
            if comments {
                println!("\n-- comments --");
                print_comments(&c, i["idReadable"].as_str().unwrap_or(&id))?;
            }
        }
        Cmd::New { project, summary, desc, field } => {
            let (pid, _short) = resolve_project(&c, &project)?;
            let desc = match desc.as_deref() {
                Some("-") => Some(stdin_text()?),
                d => d.map(String::from),
            };
            let mut body = json!({"project": {"id": pid}, "summary": summary});
            if let Some(d) = desc {
                body["description"] = json!(d);
            }
            let created = c.post("issues", &[("fields", "idReadable")], body)?;
            let id = created["idReadable"].as_str().context("create returned no id")?.to_string();
            if !field.is_empty() {
                c.post("commands", &[], json!({"query": field.join(" "), "issues": [{"idReadable": id}]}))
                    .with_context(|| format!("{id} created, but setting fields failed"))?;
            }
            println!("{id}");
        }
        Cmd::Comment { id, text } => {
            let text = match text {
                Some(t) => t,
                None => stdin_text()?,
            };
            if text.is_empty() {
                bail!("empty comment");
            }
            c.post(&format!("issues/{id}/comments"), &[], json!({"text": text}))?;
            println!("ok");
        }
        Cmd::Comments { id } => print_comments(&c, &id)?,
        Cmd::Cmd { command, ids, comment } => {
            let mut body = json!({"query": command, "issues": ids.iter().map(|i| issue_ref(i)).collect::<Vec<_>>()});
            if let Some(m) = comment {
                body["comment"] = json!(m);
            }
            c.post("commands", &[], body)?;
            println!("ok");
        }
        Cmd::Projects => {
            let projects = c.get("admin/projects", &[("fields", "shortName,name,archived"), ("$top", "500")])?;
            for p in projects.as_array().into_iter().flatten() {
                if p["archived"].as_bool() != Some(true) {
                    println!("{}  {}", p["shortName"].as_str().unwrap_or("?"), p["name"].as_str().unwrap_or(""));
                }
            }
        }
        Cmd::Fields { project } => {
            let (pid, short) = resolve_project(&c, &project)?;
            let fields = c.get(
                &format!("admin/projects/{pid}/customFields"),
                &[("fields", "canBeEmpty,field(name,fieldType(valueType)),bundle(values(name,archived))"), ("$top", "100")],
            )?;
            let field_list = fields.as_array().cloned().unwrap_or_default();
            if field_list.is_empty() {
                // field config needs project-admin rights; fall back to values observed on recent issues
                let issues = c.get(
                    "issues",
                    &[("query", &format!("project: {short}")), ("fields", "customFields(name,value(name,login,text))"), ("$top", "100")],
                )?;
                let mut seen: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> = Default::default();
                for i in issues.as_array().into_iter().flatten() {
                    for f in i["customFields"].as_array().into_iter().flatten() {
                        if let (Some(n), Some(v)) = (f["name"].as_str(), cf_value(&f["value"])) {
                            seen.entry(n.into()).or_default().insert(v);
                        }
                    }
                }
                if seen.is_empty() {
                    bail!("no field info available for {short} (no admin access, no issues to sample)");
                }
                println!("# {short}: values observed on recent issues (field config not readable with this token)");
                for (name, values) in &seen {
                    println!("{name}: {}", values.iter().cloned().collect::<Vec<_>>().join(", "));
                }
                return Ok(());
            }
            println!("# {short}: * = required");
            for f in &field_list {
                let name = f["field"]["name"].as_str().unwrap_or("?");
                let req = if f["canBeEmpty"].as_bool() == Some(false) { "*" } else { "" };
                let ty = f["field"]["fieldType"]["valueType"].as_str().unwrap_or("?");
                let values: Vec<_> = f["bundle"]["values"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|v| v["archived"].as_bool() != Some(true))
                    .filter_map(|v| v["name"].as_str())
                    .collect();
                if values.is_empty() {
                    println!("{name}{req} ({ty})");
                } else {
                    println!("{name}{req} ({ty}): {}", values.join(", "));
                }
            }
        }
        Cmd::Me => {
            let u = c.get("users/me", &[("fields", "login,name,email")])?;
            println!(
                "{}  {}  {}",
                u["login"].as_str().unwrap_or("?"),
                u["name"].as_str().unwrap_or(""),
                u["email"].as_str().unwrap_or("")
            );
        }
        Cmd::Users { query } => {
            let users = c.get("users", &[("query", &query), ("fields", "login,name"), ("$top", "10")])?;
            let list = users.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                println!("no matches");
            }
            for u in &list {
                println!("{}  {}", u["login"].as_str().unwrap_or("?"), u["name"].as_str().unwrap_or(""));
            }
        }
        Cmd::QueryHelp | Cmd::Auth { .. } => unreachable!(),
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
