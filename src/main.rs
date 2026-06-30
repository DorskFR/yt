use anstyle::{AnsiColor, Style};
use anyhow::{bail, Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use serde_json::{json, Value};
use std::io::Read;

const LIST_FIELDS: &str = "idReadable,summary,customFields(name,value(name,login,text))";
const ISSUE_FIELDS: &str = "idReadable,summary,description,created,updated,reporter(login),customFields(name,value(name,login,text))";
const COMMENT_FIELDS: &str = "created,text,author(login)";

#[derive(Parser)]
#[command(
    name = "yt",
    version,
    about = "Token-frugal YouTrack CLI (env: YOUTRACK_URL, YOUTRACK_API_TOKEN)"
)]
struct Cli {
    /// Use a named server from config (env vars still take precedence)
    #[arg(long, global = true)]
    server: Option<String>,
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
        /// Field assignment in command syntax, repeatable, e.g. -f "Priority Critical" -f "State In Progress" (field names vary by project — see `yt fields <PROJECT>`)
        #[arg(short, long)]
        field: Vec<String>,
    },
    /// Edit an issue's summary and/or description; prints the ID
    Edit {
        id: String,
        /// New summary
        #[arg(short, long)]
        summary: Option<String>,
        /// New description ("-" reads stdin)
        #[arg(short, long)]
        desc: Option<String>,
    },
    /// List an issue's attachments; -o DIR downloads them
    Attachments {
        id: String,
        /// Download all attachments to DIR (default: current directory)
        #[arg(short = 'o', long = "out")]
        out: Option<Option<String>>,
    },
    /// Attach one or more files to an issue (or a comment with -c)
    Attach {
        id: String,
        #[arg(required = true)]
        files: Vec<String>,
        /// Attach to a specific comment instead of the issue
        #[arg(short = 'c', long = "comment")]
        comment: Option<String>,
    },
    /// Add a comment (text arg, or stdin if omitted)
    Comment { id: String, text: Option<String> },
    /// List an issue's comments
    Comments { id: String },
    /// Apply a YouTrack command to issues, e.g. yt cmd "State Fixed assignee me" DEMO-1 DEMO-2
    #[allow(clippy::enum_variant_names)]
    Cmd {
        command: String,
        #[arg(required = true)]
        ids: Vec<String>,
        /// Comment to add alongside the command
        #[arg(short = 'm', long)]
        comment: Option<String>,
    },
    /// List tags (one name per line)
    Tags,
    /// Add a tag (by name) to an issue
    Tag { id: String, tag: String },
    /// Remove a tag (by name) from an issue
    Untag { id: String, tag: String },
    /// List projects
    Projects,
    /// Manage projects (admin token required for creation)
    Project {
        #[command(subcommand)]
        cmd: ProjectCmd,
    },
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
        /// Server name; defaults to "default" (warns if overwriting an existing default)
        name: Option<String>,
    },
    /// List configured servers (* marks the default)
    Servers,
    /// Set the default server
    Default {
        /// Server name
        name: String,
    },
    /// Print a shell completion script to stdout
    Completions {
        /// Target shell
        shell: Shell,
    },
}

#[derive(Subcommand)]
enum ProjectCmd {
    /// Create a project (requires an admin token); prints SHORT  ID
    New {
        /// Short name / key, e.g. DEMO
        short: String,
        /// Full project name
        name: String,
    },
}

fn config_path() -> std::path::PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .unwrap_or_default()
        .join("yt/config.json")
}

/// On-disk config: {"default": "<name>", "servers": {"<name>": {"url","token"}}}.
/// Transparently migrates the legacy {"url","token"} single-server shape.
#[derive(Default)]
struct Config {
    default: Option<String>,
    servers: std::collections::BTreeMap<String, (String, String)>,
}

impl Config {
    fn load() -> Result<Self> {
        let path = config_path();
        let Ok(s) = std::fs::read_to_string(&path) else {
            return Ok(Self::default());
        };
        let cfg: Value = serde_json::from_str(&s)
            .with_context(|| format!("invalid JSON in {}", path.display()))?;
        Ok(Self::from_value(&cfg))
    }

    /// Parse the on-disk JSON shape into a `Config`, transparently migrating the
    /// legacy single-server `{"url","token"}` layout. Pure: no IO.
    fn from_value(cfg: &Value) -> Self {
        let mut servers = std::collections::BTreeMap::new();
        if let Some(obj) = cfg["servers"].as_object() {
            for (name, v) in obj {
                if let (Some(u), Some(t)) = (v["url"].as_str(), v["token"].as_str()) {
                    servers.insert(name.clone(), (u.to_string(), t.to_string()));
                }
            }
        } else if let (Some(u), Some(t)) = (cfg["url"].as_str(), cfg["token"].as_str()) {
            // legacy single-server config
            servers.insert("default".into(), (u.to_string(), t.to_string()));
        }
        let default = cfg["default"].as_str().map(String::from);
        Self { default, servers }
    }

    /// Pick a server name given an explicit `--server`, falling back to the
    /// configured default, then to the sole server if exactly one exists. Pure.
    fn select_server(&self, server: Option<&str>) -> Result<String> {
        match server {
            Some(s) => Ok(s.to_string()),
            None => self
                .default
                .clone()
                .or_else(|| {
                    // single configured server is unambiguous
                    (self.servers.len() == 1).then(|| self.servers.keys().next().unwrap().clone())
                })
                .context("no server selected: set a default with `yt default NAME`, pass --server, or set YOUTRACK_URL"),
        }
    }

    fn save(&self) -> Result<()> {
        let path = config_path();
        std::fs::create_dir_all(path.parent().context("no config dir")?)?;
        let servers: serde_json::Map<String, Value> = self
            .servers
            .iter()
            .map(|(name, (u, t))| (name.clone(), json!({"url": u, "token": t})))
            .collect();
        let cfg = json!({"default": self.default, "servers": servers});
        std::fs::write(&path, serde_json::to_string_pretty(&cfg)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

/// Build the base API URL from a configured/env YouTrack URL: strip trailing
/// slashes and append `/api` unless it is already present. Pure.
fn normalize_base(url: &str) -> String {
    let url = url.trim_end_matches('/');
    if url.ends_with("/api") {
        url.to_string()
    } else {
        format!("{url}/api")
    }
}

struct Client {
    base: String,
    token: String,
}

impl Client {
    fn resolve(server: Option<&str>) -> Result<Self> {
        let env_url = std::env::var("YOUTRACK_URL").ok();
        let env_token = std::env::var("YOUTRACK_API_TOKEN").ok();
        let (url, token) = match (env_url, env_token) {
            // both env vars present (and no explicit --server): use them directly
            (Some(u), Some(t)) if server.is_none() => (u, t),
            _ => {
                let cfg = Config::load()?;
                let name = cfg.select_server(server)?;
                let (u, t) = cfg.servers.get(&name).with_context(|| {
                    format!("no server named '{name}': run `yt auth URL TOKEN {name}`")
                })?;
                (u.clone(), t.clone())
            }
        };
        Ok(Self {
            base: normalize_base(&url),
            token,
        })
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

    fn delete(&self, path: &str) -> Result<Value> {
        read(self.req("DELETE", path, &[]).call())
    }

    /// Host root (base minus the trailing `/api`), for building web/file URLs.
    fn host(&self) -> &str {
        self.base.strip_suffix("/api").unwrap_or(&self.base)
    }

    /// Browser URL for an issue, e.g. https://yt.example.com/issue/YT-1.
    fn web_url(&self, id: &str) -> String {
        format!("{}/issue/{}", self.host(), id)
    }

    /// Fetch raw bytes from a server-relative URL (the attachment `url` field is
    /// relative to the host root, e.g. "/api/files/..."), with the auth header.
    fn get_bytes(&self, rel_url: &str) -> Result<Vec<u8>> {
        let url = format!("{}{rel_url}", self.host());
        let res = ureq::get(&url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .call();
        match res {
            Ok(r) => {
                let mut buf = Vec::new();
                r.into_reader().read_to_end(&mut buf)?;
                Ok(buf)
            }
            Err(ureq::Error::Status(code, r)) => {
                let body = r.into_string().unwrap_or_default();
                bail!("HTTP {code}: {body}")
            }
            Err(e) => bail!(e.to_string()),
        }
    }

    /// POST a single file as multipart/form-data (field name `file`) to a
    /// server-relative API path. ureq has no multipart support, so we build the
    /// body by hand.
    fn post_file(&self, path: &str, params: &[(&str, &str)], file: &str) -> Result<Value> {
        let p = std::path::Path::new(file);
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .with_context(|| format!("bad file name: {file}"))?;
        let data = std::fs::read(p).with_context(|| format!("cannot read {file}"))?;
        let ctype = content_type(name);
        let boundary = format!("----ytboundary{:x}", data.len().wrapping_mul(2_654_435_761));
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\n\
                 Content-Type: {ctype}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(&data);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let res = self
            .req("POST", path, params)
            .set(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .send_bytes(&body);
        read(res)
    }
}

/// Infer a content-type from a file extension for common image types.
fn content_type(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn read(res: Result<ureq::Response, ureq::Error>) -> Result<Value> {
    match res {
        Ok(r) => {
            let s = r.into_string()?;
            Ok(if s.is_empty() {
                Value::Null
            } else {
                serde_json::from_str(&s)?
            })
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
    let Some(ms) = v.as_i64() else {
        return "-".into();
    };
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
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(","))
            }
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
        .find(|f| {
            f["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .and_then(|f| cf_value(&f["value"]))
}

// --- output styling -------------------------------------------------------
// Styles are emitted via anstream macros, which strip ANSI when stdout is not
// a TTY and honor NO_COLOR / CLICOLOR — so piped/agent output stays plain.

/// Issue IDs: bold cyan.
fn id_style() -> Style {
    Style::new().bold().fg_color(Some(AnsiColor::Cyan.into()))
}

/// State: green when resolved-ish, yellow when active, plain otherwise.
fn state_style(s: &str) -> Style {
    let l = s.to_ascii_lowercase();
    let color = if [
        "done", "fixed", "resolved", "verified", "closed", "complete",
    ]
    .iter()
    .any(|k| l.contains(k))
    {
        AnsiColor::Green
    } else if [
        "open", "new", "progress", "backlog", "reopen", "to do", "wait",
    ]
    .iter()
    .any(|k| l.contains(k))
    {
        AnsiColor::Yellow
    } else {
        return Style::new();
    };
    Style::new().fg_color(Some(color.into()))
}

/// Priority: red (bold for critical), dim for minor, plain otherwise.
fn prio_style(s: &str) -> Style {
    let l = s.to_ascii_lowercase();
    if l.contains("critical") || l.contains("show") || l.contains("blocker") {
        Style::new().bold().fg_color(Some(AnsiColor::Red.into()))
    } else if l.contains("major") || l.contains("urgent") {
        Style::new().fg_color(Some(AnsiColor::Red.into()))
    } else if l.contains("minor") {
        Style::new().dimmed()
    } else {
        Style::new()
    }
}

fn stdin_text() -> Result<String> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s)?;
    Ok(s.trim_end().to_string())
}

fn resolve_project(c: &Client, key: &str) -> Result<(String, String)> {
    let projects = c.get(
        "admin/projects",
        &[("fields", "id,shortName,name"), ("$top", "500")],
    )?;
    projects
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| {
            p["shortName"]
                .as_str()
                .is_some_and(|s| s.eq_ignore_ascii_case(key))
                || p["name"]
                    .as_str()
                    .is_some_and(|s| s.eq_ignore_ascii_case(key))
        })
        .map(|p| {
            (
                p["id"].as_str().unwrap_or_default().to_string(),
                p["shortName"].as_str().unwrap_or(key).to_string(),
            )
        })
        .with_context(|| format!("project not found: {key}"))
}

/// Resolve a tag name to its internal id via GET /api/tags.
fn resolve_tag(c: &Client, name: &str) -> Result<String> {
    let tags = c.get("tags", &[("fields", "id,name"), ("$top", "500")])?;
    tags.as_array()
        .into_iter()
        .flatten()
        .find(|t| {
            t["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .and_then(|t| t["id"].as_str().map(String::from))
        .with_context(|| format!("tag not found: {name}"))
}

fn issue_ref(id: &str) -> Value {
    // internal ids look like "2-123"; anything else is treated as readable (DEMO-1)
    let internal = id.split_once('-').is_some_and(|(a, b)| {
        !a.is_empty() && !b.is_empty() && a.chars().chain(b.chars()).all(|c| c.is_ascii_digit())
    });
    if internal {
        json!({"id": id})
    } else {
        json!({"idReadable": id})
    }
}

fn print_comments(c: &Client, id: &str) -> Result<()> {
    let comments = c.get(
        &format!("issues/{id}/comments"),
        &[("fields", COMMENT_FIELDS)],
    )?;
    let list = comments.as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        println!("no comments");
    }
    for cm in &list {
        println!(
            "[{} {}] {}",
            date(&cm["created"]),
            cm["author"]["login"].as_str().unwrap_or("?"),
            cm["text"].as_str().unwrap_or("").trim_end()
        );
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
    if let Cmd::Completions { shell } = cli.cmd {
        let mut cmd = Cli::command();
        let name = cmd.get_name().to_string();
        clap_complete::generate(shell, &mut cmd, name, &mut anstream::stdout());
        return Ok(());
    }
    if let Cmd::QueryHelp = cli.cmd {
        println!("{QUERY_HELP}");
        return Ok(());
    }
    if let Cmd::Auth { url, token, name } = &cli.cmd {
        let token = if token == "-" {
            stdin_text()?
        } else {
            token.clone()
        };
        let mut cfg = Config::load()?;
        let name = name.clone().unwrap_or_else(|| "default".to_string());
        if name == "default" && cfg.servers.contains_key("default") {
            eprintln!("warning: overwriting existing 'default' server (pass a name to keep both)");
        }
        cfg.servers.insert(name.clone(), (url.clone(), token));
        // first server added becomes the default
        if cfg.default.is_none() {
            cfg.default = Some(name.clone());
        }
        cfg.save()?;
        println!("saved server '{name}' to {}", config_path().display());
        return Ok(());
    }
    if let Cmd::Servers = cli.cmd {
        let cfg = Config::load()?;
        if cfg.servers.is_empty() {
            println!("no servers configured; run `yt auth URL TOKEN [name]`");
        }
        for (name, (url, _)) in &cfg.servers {
            let mark = if cfg.default.as_deref() == Some(name) {
                "*"
            } else {
                " "
            };
            println!("{mark} {name}  {url}");
        }
        return Ok(());
    }
    if let Cmd::Default { name } = &cli.cmd {
        let mut cfg = Config::load()?;
        if !cfg.servers.contains_key(name) {
            bail!("no server named '{name}': run `yt servers` to list");
        }
        cfg.default = Some(name.clone());
        cfg.save()?;
        println!("default server is now '{name}'");
        return Ok(());
    }
    let c = Client::resolve(cli.server.as_deref())?;

    match cli.cmd {
        Cmd::Ls { query, limit, full } => {
            let fields = if full {
                format!("{LIST_FIELDS},description")
            } else {
                LIST_FIELDS.into()
            };
            let issues = c.get(
                "issues",
                &[
                    ("query", &query),
                    ("fields", &fields),
                    ("$top", &limit.to_string()),
                ],
            )?;
            let list = issues.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                println!("no matches");
                return Ok(());
            }
            for i in &list {
                let id = i["idReadable"].as_str().unwrap_or("?");
                let state = cf_get(i, "State");
                let state = state.as_deref().unwrap_or("-");
                let prio = cf_get(i, "Priority");
                let prio = prio.as_deref().unwrap_or("-");
                let (ids, ss, ps) = (id_style(), state_style(state), prio_style(prio));
                anstream::println!(
                    "{ids}{id}{ids:#}  {ss}{state}{ss:#}  {ps}{prio}{ps:#}  {}",
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
            let ids = id_style();
            anstream::println!(
                "{ids}{}{ids:#}  {}",
                i["idReadable"].as_str().unwrap_or(&id),
                i["summary"].as_str().unwrap_or("")
            );
            let mut meta: Vec<String> = i["customFields"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|f| {
                    Some(format!(
                        "{}:{}",
                        f["name"].as_str()?,
                        cf_value(&f["value"])?
                    ))
                })
                .collect();
            meta.push(format!("created:{}", date(&i["created"])));
            meta.push(format!("updated:{}", date(&i["updated"])));
            if let Some(r) = i["reporter"]["login"].as_str() {
                meta.push(format!("by:{r}"));
            }
            println!("{}", meta.join("  "));
            let link = Style::new()
                .underline()
                .fg_color(Some(AnsiColor::Blue.into()));
            anstream::println!(
                "{link}{}{link:#}",
                c.web_url(i["idReadable"].as_str().unwrap_or(&id))
            );
            if let Some(d) = i["description"].as_str().filter(|d| !d.is_empty()) {
                println!("\n{}", d.trim_end());
            }
            if comments {
                println!("\n-- comments --");
                print_comments(&c, i["idReadable"].as_str().unwrap_or(&id))?;
            }
        }
        Cmd::New {
            project,
            summary,
            desc,
            field,
        } => {
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
            let id = created["idReadable"]
                .as_str()
                .context("create returned no id")?
                .to_string();
            if !field.is_empty() {
                c.post(
                    "commands",
                    &[],
                    json!({"query": field.join(" "), "issues": [{"idReadable": id}]}),
                )
                .with_context(|| format!("{id} created, but setting fields failed"))?;
            }
            println!("{id}");
        }
        Cmd::Edit { id, summary, desc } => {
            let desc = match desc.as_deref() {
                Some("-") => Some(stdin_text()?),
                d => d.map(String::from),
            };
            if summary.is_none() && desc.is_none() {
                bail!("nothing to edit: pass --summary and/or --desc");
            }
            let mut body = json!({});
            if let Some(s) = summary {
                body["summary"] = json!(s);
            }
            if let Some(d) = desc {
                body["description"] = json!(d);
            }
            c.post(&format!("issues/{id}"), &[("fields", "idReadable")], body)?;
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
        Cmd::Attachments { id, out } => {
            let atts = c.get(
                &format!("issues/{id}/attachments"),
                &[("fields", "id,name,size,url")],
            )?;
            let list = atts.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                println!("no attachments");
                return Ok(());
            }
            match out {
                None => {
                    for a in &list {
                        println!(
                            "{}  {}",
                            a["name"].as_str().unwrap_or("?"),
                            a["size"].as_i64().unwrap_or(0)
                        );
                    }
                }
                Some(dir) => {
                    let dir = dir.as_deref().unwrap_or(".");
                    std::fs::create_dir_all(dir)?;
                    for a in &list {
                        let name = a["name"].as_str().unwrap_or("attachment");
                        let url = a["url"]
                            .as_str()
                            .with_context(|| format!("attachment '{name}' has no url"))?;
                        let bytes = c.get_bytes(url)?;
                        let path = std::path::Path::new(dir).join(name);
                        std::fs::write(&path, &bytes)?;
                        println!("{}  {}", path.display(), bytes.len());
                    }
                }
            }
        }
        Cmd::Attach { id, files, comment } => {
            let path = match &comment {
                Some(cid) => format!("issues/{id}/comments/{cid}/attachments"),
                None => format!("issues/{id}/attachments"),
            };
            for f in &files {
                let a = c.post_file(&path, &[("fields", "id,name")], f)?;
                println!(
                    "{}  {}",
                    a["id"].as_str().unwrap_or("?"),
                    a["name"].as_str().unwrap_or("?")
                );
            }
        }
        Cmd::Cmd {
            command,
            ids,
            comment,
        } => {
            let mut body = json!({"query": command, "issues": ids.iter().map(|i| issue_ref(i)).collect::<Vec<_>>()});
            if let Some(m) = comment {
                body["comment"] = json!(m);
            }
            c.post("commands", &[], body)?;
            println!("ok");
        }
        Cmd::Tags => {
            let tags = c.get("tags", &[("fields", "id,name"), ("$top", "500")])?;
            let list = tags.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                println!("no tags");
            }
            for t in &list {
                println!("{}", t["name"].as_str().unwrap_or("?"));
            }
        }
        Cmd::Tag { id, tag } => {
            let tid = resolve_tag(&c, &tag)?;
            c.post(&format!("issues/{id}/tags"), &[], json!({"id": tid}))?;
            println!("ok");
        }
        Cmd::Untag { id, tag } => {
            let tid = resolve_tag(&c, &tag)?;
            c.delete(&format!("issues/{id}/tags/{tid}"))?;
            println!("ok");
        }
        Cmd::Projects => {
            let projects = c.get(
                "admin/projects",
                &[("fields", "shortName,name,archived"), ("$top", "500")],
            )?;
            for p in projects.as_array().into_iter().flatten() {
                if p["archived"].as_bool() != Some(true) {
                    println!(
                        "{}  {}",
                        p["shortName"].as_str().unwrap_or("?"),
                        p["name"].as_str().unwrap_or("")
                    );
                }
            }
        }
        Cmd::Project { cmd } => match cmd {
            ProjectCmd::New { short, name } => {
                // leader is required; resolve the current user's id to populate it
                let me = c.get("users/me", &[("fields", "id")])?;
                let leader = me["id"]
                    .as_str()
                    .context("could not resolve current user")?;
                let created = c.post(
                    "admin/projects",
                    &[("fields", "id,shortName")],
                    json!({"name": name, "shortName": short, "leader": {"id": leader}}),
                )?;
                println!(
                    "{}  {}",
                    created["shortName"].as_str().unwrap_or(&short),
                    created["id"].as_str().unwrap_or("?")
                );
            }
        },
        Cmd::Fields { project } => {
            let (pid, short) = resolve_project(&c, &project)?;
            let fields = c.get(
                &format!("admin/projects/{pid}/customFields"),
                &[
                    (
                        "fields",
                        "canBeEmpty,field(name,fieldType(valueType)),bundle(values(name,archived))",
                    ),
                    ("$top", "100"),
                ],
            )?;
            let field_list = fields.as_array().cloned().unwrap_or_default();
            if field_list.is_empty() {
                // field config needs project-admin rights; fall back to values observed on recent issues
                let issues = c.get(
                    "issues",
                    &[
                        ("query", &format!("project: {short}")),
                        ("fields", "customFields(name,value(name,login,text))"),
                        ("$top", "100"),
                    ],
                )?;
                let mut seen: std::collections::BTreeMap<
                    String,
                    std::collections::BTreeSet<String>,
                > = Default::default();
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
                    println!(
                        "{name}: {}",
                        values.iter().cloned().collect::<Vec<_>>().join(", ")
                    );
                }
                return Ok(());
            }
            println!("# {short}: * = required");
            for f in &field_list {
                let name = f["field"]["name"].as_str().unwrap_or("?");
                let req = if f["canBeEmpty"].as_bool() == Some(false) {
                    "*"
                } else {
                    ""
                };
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
            let users = c.get(
                "users",
                &[("query", &query), ("fields", "login,name"), ("$top", "10")],
            )?;
            let list = users.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                println!("no matches");
            }
            for u in &list {
                println!(
                    "{}  {}",
                    u["login"].as_str().unwrap_or("?"),
                    u["name"].as_str().unwrap_or("")
                );
            }
        }
        Cmd::QueryHelp
        | Cmd::Auth { .. }
        | Cmd::Servers
        | Cmd::Default { .. }
        | Cmd::Completions { .. } => unreachable!(),
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- URL normalization ----

    #[test]
    fn normalize_base_appends_api() {
        assert_eq!(
            normalize_base("https://yt.example.com"),
            "https://yt.example.com/api"
        );
    }

    #[test]
    fn normalize_base_strips_trailing_slash() {
        assert_eq!(
            normalize_base("https://yt.example.com/"),
            "https://yt.example.com/api"
        );
    }

    #[test]
    fn normalize_base_strips_multiple_trailing_slashes() {
        assert_eq!(
            normalize_base("https://yt.example.com///"),
            "https://yt.example.com/api"
        );
    }

    #[test]
    fn normalize_base_keeps_existing_api_suffix() {
        assert_eq!(
            normalize_base("https://yt.example.com/api"),
            "https://yt.example.com/api"
        );
    }

    #[test]
    fn normalize_base_api_suffix_with_trailing_slash() {
        assert_eq!(
            normalize_base("https://yt.example.com/api/"),
            "https://yt.example.com/api"
        );
    }

    #[test]
    fn normalize_base_subpath_gets_api() {
        // a hosted instance under a subpath should still get /api appended
        assert_eq!(
            normalize_base("https://host/youtrack"),
            "https://host/youtrack/api"
        );
    }

    // ---- config: multi-server parsing ----

    #[test]
    fn config_parses_multi_server() {
        let cfg = Config::from_value(&json!({
            "default": "work",
            "servers": {
                "work": {"url": "https://work.example.com", "token": "wtok"},
                "home": {"url": "https://home.example.com", "token": "htok"},
            }
        }));
        assert_eq!(cfg.default.as_deref(), Some("work"));
        assert_eq!(cfg.servers.len(), 2);
        assert_eq!(
            cfg.servers.get("work"),
            Some(&("https://work.example.com".into(), "wtok".into()))
        );
        assert_eq!(
            cfg.servers.get("home"),
            Some(&("https://home.example.com".into(), "htok".into()))
        );
    }

    #[test]
    fn config_skips_incomplete_server_entries() {
        let cfg = Config::from_value(&json!({
            "servers": {
                "ok": {"url": "https://ok.example.com", "token": "t"},
                "notoken": {"url": "https://x.example.com"},
                "nourl": {"token": "t"},
            }
        }));
        assert_eq!(cfg.servers.len(), 1);
        assert!(cfg.servers.contains_key("ok"));
    }

    #[test]
    fn config_empty_value_is_empty() {
        let cfg = Config::from_value(&json!({}));
        assert!(cfg.default.is_none());
        assert!(cfg.servers.is_empty());
    }

    // ---- config: legacy flat-config migration ----

    #[test]
    fn config_migrates_legacy_flat_shape() {
        let cfg = Config::from_value(&json!({
            "url": "https://legacy.example.com",
            "token": "legacytok"
        }));
        assert_eq!(cfg.servers.len(), 1);
        assert_eq!(
            cfg.servers.get("default"),
            Some(&("https://legacy.example.com".into(), "legacytok".into()))
        );
        // legacy shape carries no explicit default
        assert!(cfg.default.is_none());
    }

    #[test]
    fn config_prefers_servers_over_legacy_keys() {
        // when both shapes are present, the modern `servers` block wins
        let cfg = Config::from_value(&json!({
            "url": "https://legacy.example.com",
            "token": "legacytok",
            "servers": {
                "main": {"url": "https://main.example.com", "token": "mtok"},
            }
        }));
        assert!(cfg.servers.contains_key("main"));
        assert!(!cfg.servers.contains_key("default"));
    }

    // ---- config: server selection (env-vs-config precedence + default) ----

    fn cfg_with(default: Option<&str>, names: &[&str]) -> Config {
        let mut servers = std::collections::BTreeMap::new();
        for n in names {
            servers.insert(
                (*n).to_string(),
                (format!("https://{n}"), format!("{n}tok")),
            );
        }
        Config {
            default: default.map(String::from),
            servers,
        }
    }

    #[test]
    fn select_server_explicit_wins() {
        let cfg = cfg_with(Some("work"), &["work", "home"]);
        assert_eq!(cfg.select_server(Some("home")).unwrap(), "home");
    }

    #[test]
    fn select_server_falls_back_to_default() {
        let cfg = cfg_with(Some("work"), &["work", "home"]);
        assert_eq!(cfg.select_server(None).unwrap(), "work");
    }

    #[test]
    fn select_server_single_server_is_unambiguous() {
        let cfg = cfg_with(None, &["only"]);
        assert_eq!(cfg.select_server(None).unwrap(), "only");
    }

    #[test]
    fn select_server_ambiguous_without_default_errors() {
        let cfg = cfg_with(None, &["a", "b"]);
        assert!(cfg.select_server(None).is_err());
    }

    #[test]
    fn select_server_no_servers_errors() {
        let cfg = cfg_with(None, &[]);
        assert!(cfg.select_server(None).is_err());
    }

    // Env-vs-config precedence: when both env vars are set and no --server is
    // passed, Client::resolve uses the env values directly and never touches the
    // config file. We exercise that branch with a custom-URL env to also cover
    // normalization end-to-end. (Set/remove env in one test to avoid cross-test
    // interference since env is process-global.)
    #[test]
    fn resolve_prefers_env_over_config() {
        std::env::set_var("YOUTRACK_URL", "https://env.example.com/");
        std::env::set_var("YOUTRACK_API_TOKEN", "envtok");
        let c = Client::resolve(None).unwrap();
        std::env::remove_var("YOUTRACK_URL");
        std::env::remove_var("YOUTRACK_API_TOKEN");
        assert_eq!(c.base, "https://env.example.com/api");
        assert_eq!(c.token, "envtok");
    }

    // ---- compact output formatting helpers ----

    #[test]
    fn date_formats_epoch_millis() {
        // 2026-06-25 00:00:00 UTC
        assert_eq!(date(&json!(1_782_345_600_000i64)), "2026-06-25");
        // unix epoch
        assert_eq!(date(&json!(0i64)), "1970-01-01");
    }

    #[test]
    fn date_non_number_is_dash() {
        assert_eq!(date(&json!(null)), "-");
        assert_eq!(date(&json!("nope")), "-");
    }

    #[test]
    fn cf_value_picks_name() {
        assert_eq!(cf_value(&json!({"name": "Open"})).as_deref(), Some("Open"));
    }

    #[test]
    fn cf_value_prefers_login_over_name() {
        assert_eq!(
            cf_value(&json!({"login": "alice", "name": "Alice A"})).as_deref(),
            Some("alice")
        );
    }

    #[test]
    fn cf_value_joins_array() {
        let v = json!([{"name": "Bug"}, {"name": "Critical"}]);
        assert_eq!(cf_value(&v).as_deref(), Some("Bug,Critical"));
    }

    #[test]
    fn cf_value_empty_is_none() {
        assert_eq!(cf_value(&json!(null)), None);
        assert_eq!(cf_value(&json!([])), None);
        // an array of empties collapses to None
        assert_eq!(cf_value(&json!([null, null])), None);
    }

    #[test]
    fn cf_value_large_number_is_date() {
        // numbers above the threshold are treated as epoch-millis dates
        assert_eq!(
            cf_value(&json!(1_782_345_600_000i64)).as_deref(),
            Some("2026-06-25")
        );
    }

    #[test]
    fn cf_value_small_number_is_plain() {
        assert_eq!(cf_value(&json!(42)).as_deref(), Some("42"));
    }

    #[test]
    fn cf_get_matches_case_insensitively() {
        let issue = json!({
            "customFields": [
                {"name": "State", "value": {"name": "Open"}},
                {"name": "Priority", "value": {"name": "Critical"}},
            ]
        });
        assert_eq!(cf_get(&issue, "state").as_deref(), Some("Open"));
        assert_eq!(cf_get(&issue, "PRIORITY").as_deref(), Some("Critical"));
        assert_eq!(cf_get(&issue, "Assignee"), None);
    }

    #[test]
    fn issue_ref_distinguishes_internal_vs_readable() {
        // internal ids are all-digit "N-M"
        assert_eq!(issue_ref("2-123"), json!({"id": "2-123"}));
        // readable ids (project-prefixed) use idReadable
        assert_eq!(issue_ref("DEMO-1"), json!({"idReadable": "DEMO-1"}));
    }

    #[test]
    fn content_type_maps_extensions() {
        assert_eq!(content_type("a.png"), "image/png");
        assert_eq!(content_type("a.JPG"), "image/jpeg");
        assert_eq!(content_type("a.jpeg"), "image/jpeg");
        assert_eq!(content_type("a.svg"), "image/svg+xml");
        assert_eq!(content_type("a.bin"), "application/octet-stream");
        assert_eq!(content_type("noext"), "application/octet-stream");
    }
}
