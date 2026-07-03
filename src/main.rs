use anstyle::{AnsiColor, Style};
use anyhow::{bail, Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use serde_json::{json, Value};
use std::io::Read;

const LIST_FIELDS: &str = "idReadable,summary,customFields(name,value(name,login,text))";
const ISSUE_FIELDS: &str = "idReadable,summary,description,created,updated,reporter(login),customFields(name,value(name,login,text))";
const COMMENT_FIELDS: &str = "created,text,author(login)";
const LINK_FIELDS: &str =
    "id,direction,linkType(name,sourceToTarget,targetToSource),issues(idReadable,summary)";
// Pull requests surface in the activity stream under PullRequestChangeCategory
// (the vcsChanges collection only holds bare commits for the GitHub integration).
// Each PullRequestChange carries a PullRequestState whose *id* is the enum value
// OPEN / MERGED / DECLINED — note `name` comes back null, so match on `id`.
const PR_ACTIVITY_FIELDS: &str = "added(state(id),url)";

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

// The command tree is split into two permission tiers so an access guard can
// gate the whole CLI with two stable prefixes (`yt read` / `yt write`) instead
// of enumerating every subcommand. New subcommands inherit the correct tier for
// free by landing under the right noun. Layout: tier -> noun -> verb.
#[derive(Subcommand)]
enum Cmd {
    /// Read-only operations (guard prefix: `yt read`)
    Read {
        #[command(subcommand)]
        cmd: ReadCmd,
    },
    /// State-mutating operations (guard prefix: `yt write`)
    Write {
        #[command(subcommand)]
        cmd: WriteCmd,
    },
    /// Print a shell completion script to stdout
    Completions {
        /// Target shell
        shell: Shell,
    },
}

#[derive(Subcommand)]
enum ReadCmd {
    /// Read issues (ls, show, attachments, comments, links, tags)
    Issue {
        #[command(subcommand)]
        cmd: ReadIssueCmd,
    },
    /// Read projects (ls, fields)
    Project {
        #[command(subcommand)]
        cmd: ReadProjectCmd,
    },
    /// Read users (ls, me)
    User {
        #[command(subcommand)]
        cmd: ReadUserCmd,
    },
    /// Read local server config
    Server {
        #[command(subcommand)]
        cmd: ReadServerCmd,
    },
    /// Print query syntax cheat sheet
    QueryHelp,
}

#[derive(Subcommand)]
enum ReadIssueCmd {
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
        /// Keep only issues referenced in a MERGED pull request (one extra API
        /// call per result row — opt-in; pair with -n to bound the fan-out)
        #[arg(long)]
        merged_pr: bool,
    },
    /// Show one issue
    Show {
        id: String,
        /// Include comments
        #[arg(short, long)]
        comments: bool,
        /// Include linked pull requests (state, title, url)
        #[arg(long)]
        pr: bool,
    },
    /// List an issue's attachments; -o DIR downloads them
    Attachments {
        id: String,
        /// Download all attachments to DIR (default: current directory)
        #[arg(short = 'o', long = "out")]
        out: Option<Option<String>>,
    },
    /// List an issue's comments
    Comments { id: String },
    /// List an issue's links to other issues, grouped by relation
    Links { id: String },
    /// List tags (one name per line)
    Tags,
}

#[derive(Subcommand)]
enum ReadProjectCmd {
    /// List projects
    Ls,
    /// Show a project's custom fields and allowed values
    Fields { project: String },
}

#[derive(Subcommand)]
enum ReadUserCmd {
    /// Search users by name/login
    Ls { query: String },
    /// Show the authenticated user
    Me,
}

#[derive(Subcommand)]
enum ReadServerCmd {
    /// List configured servers (* marks the default)
    Ls,
}

#[derive(Subcommand)]
enum WriteCmd {
    /// Mutate issues (new, edit, attach, comment, link, unlink, cmd, tag, untag)
    Issue {
        #[command(subcommand)]
        cmd: WriteIssueCmd,
    },
    /// Mutate projects (create)
    Project {
        #[command(subcommand)]
        cmd: WriteProjectCmd,
    },
    /// Mutate local server config (default, auth)
    Server {
        #[command(subcommand)]
        cmd: WriteServerCmd,
    },
    /// Update yt to the latest release (downloads from GitHub, verifies sha256)
    Update {
        /// Reinstall the latest even if already up to date
        #[arg(short, long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum WriteIssueCmd {
    /// Create an issue; prints the new ID only
    New {
        /// Project short name or name, e.g. DEMO
        project: String,
        summary: String,
        /// Description ("-" reads stdin)
        #[arg(short, long)]
        desc: Option<String>,
        /// Field assignment in command syntax, repeatable, e.g. -f "Priority Critical" -f "State In Progress" (field names vary by project — see `yt read project fields <PROJECT>`)
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
    /// Link two issues, e.g. yt write issue link YT-1 "relates to" YT-2 (run `yt read issue links <id>` for the phrases this server accepts)
    Link {
        id: String,
        /// Relation phrase, e.g. "relates to", "depends on", "subtask of"
        phrase: String,
        target: String,
    },
    /// Remove a link between two issues (same phrase as `yt write issue link`)
    Unlink {
        id: String,
        phrase: String,
        target: String,
    },
    /// Apply a YouTrack command to issues, e.g. yt write issue cmd "State Fixed assignee me" DEMO-1 DEMO-2
    #[allow(clippy::enum_variant_names)]
    Cmd {
        command: String,
        #[arg(required = true)]
        ids: Vec<String>,
        /// Comment to add alongside the command
        #[arg(short = 'm', long)]
        comment: Option<String>,
    },
    /// Add a tag (by name) to an issue
    Tag { id: String, tag: String },
    /// Remove a tag (by name) from an issue
    Untag { id: String, tag: String },
}

#[derive(Subcommand)]
enum WriteProjectCmd {
    /// Create a project (requires an admin token); prints SHORT  ID
    Create {
        /// Short name / key, e.g. DEMO
        short: String,
        /// Full project name
        name: String,
    },
}

#[derive(Subcommand)]
enum WriteServerCmd {
    /// Save credentials to ~/.config/yt/config.json (env vars still take precedence)
    Auth {
        /// YouTrack base URL, e.g. https://youtrack.example.com
        url: String,
        /// Permanent API token ("-" reads stdin)
        token: String,
        /// Server name; defaults to "default" (warns if overwriting an existing default)
        name: Option<String>,
    },
    /// Set the default server
    Default {
        /// Server name
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

    /// Scheme + host origin (no path), e.g. https://yt.example.com. The base may
    /// carry a path prefix (e.g. .../youtrack/api); attachment `url` fields are
    /// relative to the domain root and already include that prefix.
    fn origin(&self) -> &str {
        let after_scheme = self.base.find("://").map_or(0, |i| i + 3);
        match self.base[after_scheme..].find('/') {
            Some(slash) => &self.base[..after_scheme + slash],
            None => &self.base,
        }
    }

    /// Browser URL for an issue, e.g. https://yt.example.com/issue/YT-1.
    fn web_url(&self, id: &str) -> String {
        format!("{}/issue/{}", self.host(), id)
    }

    /// Fetch raw bytes from a root-relative URL (the attachment `url` field is
    /// relative to the domain origin and already includes any path prefix, e.g.
    /// "/youtrack/api/files/..."), with the auth header.
    fn get_bytes(&self, rel_url: &str) -> Result<Vec<u8>> {
        let url = format!("{}{rel_url}", self.origin());
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

/// Build one `customFields` entry for the create body from a field name, the
/// concrete `IssueCustomField` `$type` (e.g. `SingleEnumIssueCustomField`,
/// `StateIssueCustomField`, `PeriodIssueCustomField`), and the raw value text.
/// Returns `None` for types we can't represent from a plain string (dates, or
/// anything unrecognized) so the caller can fall back to the command endpoint.
fn issue_cf_entry(name: &str, cf_type: &str, raw: &str) -> Option<Value> {
    let val = raw.trim();
    if val.is_empty() {
        return None;
    }
    let multi = cf_type.starts_with("Multi");
    // Bundle/user/owned/version/build/group fields carry an object value; multi
    // variants wrap it in an array. `wrap` applies that shape once we pick the key.
    let wrap = |one: Value| if multi { json!([one]) } else { one };
    let value = match cf_type {
        "SingleEnumIssueCustomField"
        | "MultiEnumIssueCustomField"
        | "SingleOwnedIssueCustomField"
        | "MultiOwnedIssueCustomField"
        | "SingleVersionIssueCustomField"
        | "MultiVersionIssueCustomField"
        | "SingleBuildIssueCustomField"
        | "MultiBuildIssueCustomField"
        | "SingleGroupIssueCustomField"
        | "MultiGroupIssueCustomField" => wrap(json!({"name": val})),
        "StateIssueCustomField" | "StateMachineIssueCustomField" => json!({"name": val}),
        "SingleUserIssueCustomField" | "MultiUserIssueCustomField" => wrap(json!({"login": val})),
        "TextIssueCustomField" => json!({"text": val}),
        "PeriodIssueCustomField" => json!({"presentation": val}),
        // integer / float / string all use SimpleIssueCustomField; infer the JSON
        // scalar so numeric fields don't get a string value the server rejects.
        "SimpleIssueCustomField" => {
            if let Ok(i) = val.parse::<i64>() {
                json!(i)
            } else if let Ok(f) = val.parse::<f64>() {
                json!(f)
            } else {
                json!(val)
            }
        }
        // DateIssueCustomField (epoch-millis) and any unknown type: fall back.
        _ => return None,
    };
    Some(json!({"name": name, "$type": cf_type, "value": value}))
}

/// Map an admin `fieldType.id` (`enum[1]`, `user[*]`, `period[1]`, `integer`, …)
/// to the concrete `IssueCustomField` `$type` used in the create body.
fn fieldtype_id_to_cf_type(type_id: &str) -> Option<&'static str> {
    let (base, multi) = match type_id.strip_suffix("[*]") {
        Some(b) => (b, true),
        None => (type_id.strip_suffix("[1]").unwrap_or(type_id), false),
    };
    Some(match (base, multi) {
        ("enum", false) => "SingleEnumIssueCustomField",
        ("enum", true) => "MultiEnumIssueCustomField",
        ("state", _) => "StateIssueCustomField",
        ("user", false) => "SingleUserIssueCustomField",
        ("user", true) => "MultiUserIssueCustomField",
        ("ownedField", false) => "SingleOwnedIssueCustomField",
        ("ownedField", true) => "MultiOwnedIssueCustomField",
        ("version", false) => "SingleVersionIssueCustomField",
        ("version", true) => "MultiVersionIssueCustomField",
        ("build", false) => "SingleBuildIssueCustomField",
        ("build", true) => "MultiBuildIssueCustomField",
        ("group", false) => "SingleGroupIssueCustomField",
        ("group", true) => "MultiGroupIssueCustomField",
        ("text", _) => "TextIssueCustomField",
        ("period", _) => "PeriodIssueCustomField",
        ("integer" | "float" | "string", _) => "SimpleIssueCustomField",
        _ => return None,
    })
}

/// Discover a project's custom fields as `(name, IssueCustomField $type)` pairs.
/// Prefers the admin field config (precise, needs project-admin rights); falls
/// back to sampling `$type` off recent issues, which only needs issue-read access
/// and so works with a plain reporter token.
fn project_field_types(c: &Client, pid: &str, short: &str) -> Vec<(String, String)> {
    if let Ok(cfg) = c.get(
        &format!("admin/projects/{pid}/customFields"),
        &[("fields", "field(name,fieldType(id))"), ("$top", "200")],
    ) {
        let defs: Vec<(String, String)> = cfg
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| {
                let n = f["field"]["name"].as_str()?.to_string();
                let t = f["field"]["fieldType"]["id"].as_str()?;
                Some((n, fieldtype_id_to_cf_type(t)?.to_string()))
            })
            .collect();
        if !defs.is_empty() {
            return defs;
        }
    }
    // Fallback: observe the concrete $type on fields of recent issues.
    let mut seen: std::collections::BTreeMap<String, String> = Default::default();
    if let Ok(issues) = c.get(
        "issues",
        &[
            ("query", &format!("project: {short}")),
            ("fields", "customFields($type,name)"),
            ("$top", "100"),
        ],
    ) {
        for i in issues.as_array().into_iter().flatten() {
            for f in i["customFields"].as_array().into_iter().flatten() {
                if let (Some(n), Some(t)) = (f["name"].as_str(), f["$type"].as_str()) {
                    seen.entry(n.to_string()).or_insert_with(|| t.to_string());
                }
            }
        }
    }
    seen.into_iter().collect()
}

/// Turn `-f` command-syntax specs (e.g. `"State In Progress"`, `"Priority Critical"`)
/// into a native `customFields` array for the create body, resolving each field's
/// type from the project. Returns `None` if the types can't be discovered, a field
/// name doesn't match, or any value can't be represented — signalling the caller to
/// fall back to the two-step `commands` path (never a regression).
fn resolve_custom_fields(c: &Client, pid: &str, short: &str, fields: &[String]) -> Option<Value> {
    let defs = project_field_types(c, pid, short);
    if defs.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(fields.len());
    for spec in fields {
        let spec_l = spec.to_ascii_lowercase();
        // Longest field name that is a whole-word prefix of the spec wins, so
        // "Sprint Board" is preferred over "Sprint" when both exist.
        let (name, cf_type) = defs
            .iter()
            .filter(|(n, _)| {
                let nl = n.to_ascii_lowercase();
                spec_l == nl || spec_l.starts_with(&format!("{nl} "))
            })
            .max_by_key(|(n, _)| n.len())?;
        let value = spec.get(name.len()..).unwrap_or_default();
        out.push(issue_cf_entry(name, cf_type, value)?);
    }
    Some(json!(out))
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

/// Extract pull-request state changes (state, optional url) from a
/// PullRequestChangeCategory activity payload, in chronological order. Each
/// activity item's `added` holds PullRequestChange entries whose `state.id` is
/// the PullRequestState (OPEN/MERGED/DECLINED). Pure.
fn pr_changes_from_activities(acts: &Value) -> Vec<(String, Option<String>)> {
    acts.as_array()
        .into_iter()
        .flatten()
        .flat_map(|item| item["added"].as_array().into_iter().flatten())
        .filter_map(|ch| {
            let state = ch["state"]["id"].as_str()?.to_string();
            Some((state, ch["url"].as_str().map(String::from)))
        })
        .collect()
}

/// True when any pull-request change reached MERGED (case-insensitive). OPEN and
/// DECLINED do not count. Pure.
fn has_merged_pr(changes: &[(String, Option<String>)]) -> bool {
    changes
        .iter()
        .any(|(s, _)| s.eq_ignore_ascii_case("merged"))
}

/// Fetch an issue's pull-request state changes from the activity stream, as
/// (state, optional url) tuples. One small request scoped to the PR category.
fn fetch_pr_changes(c: &Client, id: &str) -> Result<Vec<(String, Option<String>)>> {
    let acts = c.get(
        &format!("issues/{id}/activities"),
        &[
            ("categories", "PullRequestChangeCategory"),
            ("fields", PR_ACTIVITY_FIELDS),
        ],
    )?;
    Ok(pr_changes_from_activities(&acts))
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

/// The relation phrase to display for a link group, picked by its direction:
/// outward (and the undirected "both") reads source→target, inward reads back.
fn link_phrase(group: &Value) -> &str {
    let lt = &group["linkType"];
    match group["direction"].as_str() {
        Some("INWARD") => lt["targetToSource"].as_str().unwrap_or(""),
        _ => lt["sourceToTarget"].as_str().unwrap_or(""),
    }
}

/// Fetch an issue's links, keeping only groups that actually contain issues.
fn fetch_links(c: &Client, id: &str) -> Result<Vec<Value>> {
    let links = c.get(&format!("issues/{id}/links"), &[("fields", LINK_FIELDS)])?;
    Ok(links
        .as_array()
        .into_iter()
        .flatten()
        .filter(|g| g["issues"].as_array().is_some_and(|a| !a.is_empty()))
        .cloned()
        .collect())
}

/// Print link groups, one linked issue per line: `phrase  ID  summary`.
fn print_link_groups(groups: &[Value]) {
    let ids = id_style();
    for g in groups {
        let phrase = link_phrase(g);
        for li in g["issues"].as_array().into_iter().flatten() {
            anstream::println!(
                "{phrase}  {ids}{}{ids:#}  {}",
                li["idReadable"].as_str().unwrap_or("?"),
                li["summary"].as_str().unwrap_or("")
            );
        }
    }
}

/// Print an issue's links, one linked issue per line: `phrase  ID  summary`.
fn print_links(c: &Client, id: &str) -> Result<()> {
    let groups = fetch_links(c, id)?;
    if groups.is_empty() {
        println!("no links");
        return Ok(());
    }
    print_link_groups(&groups);
    Ok(())
}

/// Find the link-group id whose relation phrase matches `phrase`
/// (case-insensitive). Lists the server's accepted phrases on a miss.
fn resolve_link_group(c: &Client, id: &str, phrase: &str) -> Result<String> {
    let links = c.get(&format!("issues/{id}/links"), &[("fields", LINK_FIELDS)])?;
    let groups: Vec<Value> = links.as_array().cloned().unwrap_or_default();
    if let Some(g) = groups
        .iter()
        .find(|g| link_phrase(g).eq_ignore_ascii_case(phrase))
    {
        return Ok(g["id"].as_str().unwrap_or_default().to_string());
    }
    let mut phrases: Vec<&str> = groups
        .iter()
        .map(link_phrase)
        .filter(|p| !p.is_empty())
        .collect();
    phrases.sort_unstable();
    phrases.dedup();
    bail!(
        "unknown link phrase {phrase:?}; this server accepts: {}",
        phrases.join(", ")
    )
}

/// Resolve an issue's readable id to its internal database id (needed to
/// address a link's target on delete).
fn internal_id(c: &Client, id: &str) -> Result<String> {
    let i = c.get(&format!("issues/{id}"), &[("fields", "id")])?;
    i["id"]
        .as_str()
        .map(String::from)
        .with_context(|| format!("no such issue: {id}"))
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

// --- self-update / update notice -----------------------------------------

const REPO: &str = "DorskFR/yt";
const UPDATE_CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// Parse a `MAJOR.MINOR.PATCH` (optionally `v`-prefixed) version into a tuple
/// for ordering. Trailing pre-release/build metadata is ignored. Pure.
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches('v');
    // drop any -prerelease/+build suffix
    let core = v.split(['-', '+']).next().unwrap_or(v);
    let mut it = core.split('.');
    let maj = it.next()?.parse().ok()?;
    let min = it.next().unwrap_or("0").parse().ok()?;
    let pat = it.next().unwrap_or("0").parse().ok()?;
    Some((maj, min, pat))
}

/// Is `latest` strictly newer than `current`? Unparseable versions => false. Pure.
fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// GitHub release-asset name for the host platform, e.g. `yt-linux-amd64`.
/// None on unsupported os/arch. Pure (reads compile-time consts).
fn asset_name() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "yt-linux-amd64",
        ("linux", "aarch64") => "yt-linux-arm64",
        ("macos", "aarch64") => "yt-darwin-arm64",
        ("macos", "x86_64") => "yt-darwin-amd64",
        _ => return None,
    })
}

fn update_cache_path() -> std::path::PathBuf {
    config_path().with_file_name("update-check.json")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Query GitHub for the latest release; returns (tag, version-without-v).
/// Time-boxed; any failure is an error the caller can swallow.
fn fetch_latest_release() -> Result<(String, String)> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let res = ureq::get(&url)
        .set("User-Agent", &format!("yt/{}", env!("CARGO_PKG_VERSION")))
        .set("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(4))
        .call();
    let v = read(res)?;
    let tag = v["tag_name"]
        .as_str()
        .context("release has no tag_name")?
        .to_string();
    let version = tag.trim_start_matches('v').to_string();
    Ok((tag, version))
}

/// Best-effort: print a one-line notice to stderr when a newer release exists.
/// Uses a cached check (refreshed at most once per interval) so most runs do no
/// network IO. Silent on any failure, when opted out, or when not a TTY.
fn maybe_print_update_notice() {
    if std::env::var_os("YT_NO_UPDATE_CHECK").is_some() {
        return;
    }
    // Only nag interactive users; never pollute piped/agent output.
    use std::io::IsTerminal;
    if !std::io::stderr().is_terminal() {
        return;
    }
    let current = env!("CARGO_PKG_VERSION");
    let path = update_cache_path();
    let cached: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let checked_at = cached["checked_at"].as_u64().unwrap_or(0);
    let mut latest = cached["latest"].as_str().unwrap_or("").to_string();

    if now_secs().saturating_sub(checked_at) >= UPDATE_CHECK_INTERVAL_SECS {
        // stale (or absent): refresh in the foreground but time-boxed.
        if let Ok((_, v)) = fetch_latest_release() {
            latest = v.clone();
            let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
            let _ = std::fs::write(
                &path,
                serde_json::to_string(&json!({"checked_at": now_secs(), "latest": v}))
                    .unwrap_or_default(),
            );
        }
    }

    if !latest.is_empty() && is_newer(&latest, current) {
        eprintln!("update available: {current} -> {latest} (run: yt update)");
    }
}

/// Download the latest release binary for this platform, verify its sha256
/// against the release `SHA256SUMS`, and atomically replace the running exe.
fn self_update(force: bool) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let asset = asset_name().with_context(|| {
        format!(
            "unsupported platform: {}/{} (no prebuilt binary)",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let (tag, latest) = fetch_latest_release()?;
    if !force && !is_newer(&latest, current) {
        println!("already up to date ({current})");
        return Ok(());
    }

    let base = format!("https://github.com/{REPO}/releases/download/{tag}");
    let ua = format!("yt/{current}");

    // Fetch the checksums file and find the line for our asset.
    let sums = ureq::get(&format!("{base}/SHA256SUMS"))
        .set("User-Agent", &ua)
        .timeout(std::time::Duration::from_secs(30))
        .call();
    let sums = match sums {
        Ok(r) => r.into_string()?,
        Err(e) => bail!("fetching SHA256SUMS: {e}"),
    };
    let want = sums
        .lines()
        .find_map(|l| {
            let (hash, name) = l.split_once(char::is_whitespace)?;
            name.trim().ends_with(asset).then(|| hash.to_string())
        })
        .with_context(|| format!("no checksum for {asset} in SHA256SUMS"))?;

    // Download the binary into memory and verify before touching disk.
    eprintln!("downloading {asset} {latest}...");
    let bytes = {
        let res = ureq::get(&format!("{base}/{asset}"))
            .set("User-Agent", &ua)
            .timeout(std::time::Duration::from_secs(120))
            .call();
        match res {
            Ok(r) => {
                let mut buf = Vec::new();
                r.into_reader().read_to_end(&mut buf)?;
                buf
            }
            Err(e) => bail!("downloading {asset}: {e}"),
        }
    };

    use sha2::{Digest, Sha256};
    let got = format!("{:x}", Sha256::digest(&bytes));
    if !got.eq_ignore_ascii_case(&want) {
        bail!("checksum mismatch for {asset}: expected {want}, got {got} (aborting, binary untouched)");
    }

    // Atomic-ish replace: write next to the current exe, then rename over it.
    // rename(2) swaps the inode, so a running process keeps its old text pages.
    let exe = std::env::current_exe().context("cannot locate current executable")?;
    let dir = exe.parent().context("executable has no parent dir")?;
    let tmp = dir.join(format!(".yt-update-{}", std::process::id()));
    std::fs::write(&tmp, &bytes).with_context(|| {
        format!(
            "cannot write to {} — is the install dir writable? (try with appropriate permissions)",
            dir.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    if let Err(e) = std::fs::rename(&tmp, &exe) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("replacing {}", exe.display()));
    }
    println!("updated {current} -> {latest}");
    Ok(())
}

/// Commands that operate purely on local state (completion scripts, the query
/// cheat sheet, self-update, and the credential store) and so never build an API
/// client. Handled before `Client::resolve`. Returns true when it handled `cmd`.
fn run_local(cmd: &Cmd) -> Result<bool> {
    match cmd {
        Cmd::Completions { shell } => {
            let mut command = Cli::command();
            let name = command.get_name().to_string();
            clap_complete::generate(*shell, &mut command, name, &mut anstream::stdout());
        }
        Cmd::Read {
            cmd: ReadCmd::QueryHelp,
        } => println!("{QUERY_HELP}"),
        Cmd::Write {
            cmd: WriteCmd::Update { force },
        } => self_update(*force)?,
        Cmd::Read {
            cmd: ReadCmd::Server {
                cmd: ReadServerCmd::Ls,
            },
        } => {
            let cfg = Config::load()?;
            if cfg.servers.is_empty() {
                println!("no servers configured; run `yt write server auth URL TOKEN [name]`");
            }
            for (name, (url, _)) in &cfg.servers {
                let mark = if cfg.default.as_deref() == Some(name) {
                    "*"
                } else {
                    " "
                };
                println!("{mark} {name}  {url}");
            }
        }
        Cmd::Write {
            cmd:
                WriteCmd::Server {
                    cmd: WriteServerCmd::Auth { url, token, name },
                },
        } => {
            let token = if token == "-" {
                stdin_text()?
            } else {
                token.clone()
            };
            let mut cfg = Config::load()?;
            let name = name.clone().unwrap_or_else(|| "default".to_string());
            if name == "default" && cfg.servers.contains_key("default") {
                eprintln!(
                    "warning: overwriting existing 'default' server (pass a name to keep both)"
                );
            }
            cfg.servers.insert(name.clone(), (url.clone(), token));
            // first server added becomes the default
            if cfg.default.is_none() {
                cfg.default = Some(name.clone());
            }
            cfg.save()?;
            println!("saved server '{name}' to {}", config_path().display());
        }
        Cmd::Write {
            cmd:
                WriteCmd::Server {
                    cmd: WriteServerCmd::Default { name },
                },
        } => {
            let mut cfg = Config::load()?;
            if !cfg.servers.contains_key(name) {
                bail!("no server named '{name}': run `yt read server ls` to list");
            }
            cfg.default = Some(name.clone());
            cfg.save()?;
            println!("default server is now '{name}'");
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    if run_local(&cli.cmd)? {
        return Ok(());
    }
    let c = Client::resolve(cli.server.as_deref())?;

    // The remaining commands all hit the API. Local-only variants (completions,
    // query-help, server config) were handled by `run_local` above.
    match cli.cmd {
        Cmd::Read {
            cmd:
                ReadCmd::Issue {
                    cmd:
                        ReadIssueCmd::Ls {
                            query,
                            limit,
                            full,
                            merged_pr,
                        },
                },
        } => {
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
            let fetched = issues.as_array().cloned().unwrap_or_default();
            // --merged-pr post-filters the page with one vcsChanges call per row.
            // It filters *after* $top, so -n caps API calls, not surviving rows.
            let list: Vec<Value> = if merged_pr {
                let mut kept = Vec::new();
                for i in &fetched {
                    let id = i["idReadable"].as_str().unwrap_or_default();
                    if has_merged_pr(&fetch_pr_changes(&c, id)?) {
                        kept.push(i.clone());
                    }
                }
                kept
            } else {
                fetched.clone()
            };
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
            if fetched.len() == limit {
                println!("# limit {limit} reached; refine query or raise -n");
            }
        }
        Cmd::Read {
            cmd:
                ReadCmd::Issue {
                    cmd: ReadIssueCmd::Show { id, comments, pr },
                },
        } => {
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
            let links = fetch_links(&c, i["idReadable"].as_str().unwrap_or(&id))?;
            if !links.is_empty() {
                println!("\n-- links --");
                print_link_groups(&links);
            }
            if pr {
                let prs = fetch_pr_changes(&c, i["idReadable"].as_str().unwrap_or(&id))?;
                println!("\n-- pull requests --");
                if prs.is_empty() {
                    println!("no pull requests");
                }
                for (state, url) in &prs {
                    let ss = state_style(state);
                    anstream::println!("{ss}{state}{ss:#}  {}", url.as_deref().unwrap_or(""));
                }
            }
            if comments {
                println!("\n-- comments --");
                print_comments(&c, i["idReadable"].as_str().unwrap_or(&id))?;
            }
        }
        Cmd::Write {
            cmd:
                WriteCmd::Issue {
                    cmd:
                        WriteIssueCmd::New {
                            project,
                            summary,
                            desc,
                            field,
                        },
                },
        } => {
            let (pid, short) = resolve_project(&c, &project)?;
            let desc = match desc.as_deref() {
                Some("-") => Some(stdin_text()?),
                d => d.map(String::from),
            };
            let mut body = json!({"project": {"id": pid}, "summary": summary});
            if let Some(d) = desc {
                body["description"] = json!(d);
            }
            // Prefer setting custom fields atomically in the create call, so no
            // separate mutation fires project workflows (e.g. auto-assign on state
            // change) and there's no created-but-unconfigured partial-failure window.
            // If any field can't be resolved to a native type, fall back below.
            let embedded = if field.is_empty() {
                false
            } else if let Some(cf) = resolve_custom_fields(&c, &pid, &short, &field) {
                body["customFields"] = cf;
                true
            } else {
                false
            };
            let created = c.post("issues", &[("fields", "idReadable")], body)?;
            let id = created["idReadable"]
                .as_str()
                .context("create returned no id")?
                .to_string();
            if !field.is_empty() && !embedded {
                // Fallback: forgiving command-syntax endpoint (a second request).
                c.post(
                    "commands",
                    &[],
                    json!({"query": field.join(" "), "issues": [{"idReadable": id}]}),
                )
                .with_context(|| format!("{id} created, but setting fields failed"))?;
            }
            println!("{id}");
        }
        Cmd::Write {
            cmd:
                WriteCmd::Issue {
                    cmd: WriteIssueCmd::Edit { id, summary, desc },
                },
        } => {
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
        Cmd::Write {
            cmd:
                WriteCmd::Issue {
                    cmd: WriteIssueCmd::Comment { id, text },
                },
        } => {
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
        Cmd::Read {
            cmd: ReadCmd::Issue {
                cmd: ReadIssueCmd::Comments { id },
            },
        } => print_comments(&c, &id)?,
        Cmd::Read {
            cmd: ReadCmd::Issue {
                cmd: ReadIssueCmd::Links { id },
            },
        } => print_links(&c, &id)?,
        Cmd::Write {
            cmd:
                WriteCmd::Issue {
                    cmd: WriteIssueCmd::Link { id, phrase, target },
                },
        } => {
            let group = resolve_link_group(&c, &id, &phrase)?;
            c.post(
                &format!("issues/{id}/links/{group}/issues"),
                &[("fields", "idReadable")],
                json!({"idReadable": target}),
            )?;
            println!("{id} {phrase} {target}");
        }
        Cmd::Write {
            cmd:
                WriteCmd::Issue {
                    cmd: WriteIssueCmd::Unlink { id, phrase, target },
                },
        } => {
            let group = resolve_link_group(&c, &id, &phrase)?;
            let tid = internal_id(&c, &target)?;
            c.delete(&format!("issues/{id}/links/{group}/issues/{tid}"))?;
            println!("{id} unlinked {target}");
        }
        Cmd::Read {
            cmd:
                ReadCmd::Issue {
                    cmd: ReadIssueCmd::Attachments { id, out },
                },
        } => {
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
        Cmd::Write {
            cmd:
                WriteCmd::Issue {
                    cmd: WriteIssueCmd::Attach { id, files, comment },
                },
        } => {
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
        Cmd::Write {
            cmd:
                WriteCmd::Issue {
                    cmd:
                        WriteIssueCmd::Cmd {
                            command,
                            ids,
                            comment,
                        },
                },
        } => {
            let mut body = json!({"query": command, "issues": ids.iter().map(|i| issue_ref(i)).collect::<Vec<_>>()});
            if let Some(m) = comment {
                body["comment"] = json!(m);
            }
            c.post("commands", &[], body)?;
            println!("ok");
        }
        Cmd::Read {
            cmd: ReadCmd::Issue {
                cmd: ReadIssueCmd::Tags,
            },
        } => {
            let tags = c.get("tags", &[("fields", "id,name"), ("$top", "500")])?;
            let list = tags.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                println!("no tags");
            }
            for t in &list {
                println!("{}", t["name"].as_str().unwrap_or("?"));
            }
        }
        Cmd::Write {
            cmd:
                WriteCmd::Issue {
                    cmd: WriteIssueCmd::Tag { id, tag },
                },
        } => {
            let tid = resolve_tag(&c, &tag)?;
            c.post(&format!("issues/{id}/tags"), &[], json!({"id": tid}))?;
            println!("ok");
        }
        Cmd::Write {
            cmd:
                WriteCmd::Issue {
                    cmd: WriteIssueCmd::Untag { id, tag },
                },
        } => {
            let tid = resolve_tag(&c, &tag)?;
            c.delete(&format!("issues/{id}/tags/{tid}"))?;
            println!("ok");
        }
        Cmd::Read {
            cmd: ReadCmd::Project {
                cmd: ReadProjectCmd::Ls,
            },
        } => {
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
        Cmd::Write {
            cmd:
                WriteCmd::Project {
                    cmd: WriteProjectCmd::Create { short, name },
                },
        } => {
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
        Cmd::Read {
            cmd:
                ReadCmd::Project {
                    cmd: ReadProjectCmd::Fields { project },
                },
        } => {
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
        Cmd::Read {
            cmd: ReadCmd::User {
                cmd: ReadUserCmd::Me,
            },
        } => {
            let u = c.get("users/me", &[("fields", "login,name,email")])?;
            println!(
                "{}  {}  {}",
                u["login"].as_str().unwrap_or("?"),
                u["name"].as_str().unwrap_or(""),
                u["email"].as_str().unwrap_or("")
            );
        }
        Cmd::Read {
            cmd: ReadCmd::User {
                cmd: ReadUserCmd::Ls { query },
            },
        } => {
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
        // Local-only commands handled in `run_local` before the client resolves.
        Cmd::Completions { .. }
        | Cmd::Read {
            cmd: ReadCmd::QueryHelp,
        }
        | Cmd::Read {
            cmd: ReadCmd::Server { .. },
        }
        | Cmd::Write {
            cmd: WriteCmd::Update { .. },
        }
        | Cmd::Write {
            cmd: WriteCmd::Server { .. },
        } => unreachable!(),
    }
    maybe_print_update_notice();
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
    fn link_phrase_picks_direction() {
        let lt = json!({"sourceToTarget": "is required for", "targetToSource": "depends on"});
        let outward = json!({"direction": "OUTWARD", "linkType": lt});
        let inward = json!({"direction": "INWARD", "linkType": lt});
        let both = json!({"direction": "BOTH", "linkType": {"sourceToTarget": "relates to", "targetToSource": ""}});
        assert_eq!(link_phrase(&outward), "is required for");
        assert_eq!(link_phrase(&inward), "depends on");
        assert_eq!(link_phrase(&both), "relates to");
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
    fn issue_cf_entry_single_enum() {
        assert_eq!(
            issue_cf_entry("Priority", "SingleEnumIssueCustomField", "Critical"),
            Some(json!({
                "name": "Priority",
                "$type": "SingleEnumIssueCustomField",
                "value": {"name": "Critical"}
            }))
        );
    }

    #[test]
    fn issue_cf_entry_state_and_multi_and_user() {
        // state is always single, object value
        assert_eq!(
            issue_cf_entry("State", "StateIssueCustomField", "In Progress"),
            Some(json!({
                "name": "State",
                "$type": "StateIssueCustomField",
                "value": {"name": "In Progress"}
            }))
        );
        // multi wraps the value in an array and uses the Multi* type
        assert_eq!(
            issue_cf_entry("Tags", "MultiEnumIssueCustomField", "backend"),
            Some(json!({
                "name": "Tags",
                "$type": "MultiEnumIssueCustomField",
                "value": [{"name": "backend"}]
            }))
        );
        // user fields key on login
        assert_eq!(
            issue_cf_entry("Assignee", "SingleUserIssueCustomField", "shoko"),
            Some(json!({
                "name": "Assignee",
                "$type": "SingleUserIssueCustomField",
                "value": {"login": "shoko"}
            }))
        );
    }

    #[test]
    fn issue_cf_entry_scalars_and_unsupported() {
        // integer parses to a JSON number
        assert_eq!(
            issue_cf_entry("Estimation", "SimpleIssueCustomField", "5"),
            Some(json!({"name": "Estimation", "$type": "SimpleIssueCustomField", "value": 5}))
        );
        // non-numeric into a simple field is kept as a string (server validates)
        assert_eq!(
            issue_cf_entry("Note", "SimpleIssueCustomField", "soon"),
            Some(json!({"name": "Note", "$type": "SimpleIssueCustomField", "value": "soon"}))
        );
        // dates fall back to the command endpoint
        assert_eq!(
            issue_cf_entry("Due Date", "DateIssueCustomField", "2026-07-03"),
            None
        );
        // empty value -> None
        assert_eq!(
            issue_cf_entry("Priority", "SingleEnumIssueCustomField", "  "),
            None
        );
    }

    #[test]
    fn fieldtype_id_maps_to_issue_cf_type() {
        assert_eq!(
            fieldtype_id_to_cf_type("enum[1]"),
            Some("SingleEnumIssueCustomField")
        );
        assert_eq!(
            fieldtype_id_to_cf_type("user[*]"),
            Some("MultiUserIssueCustomField")
        );
        assert_eq!(
            fieldtype_id_to_cf_type("state[1]"),
            Some("StateIssueCustomField")
        );
        assert_eq!(
            fieldtype_id_to_cf_type("integer"),
            Some("SimpleIssueCustomField")
        );
        assert_eq!(fieldtype_id_to_cf_type("date"), None);
    }

    #[test]
    fn issue_ref_distinguishes_internal_vs_readable() {
        // internal ids are all-digit "N-M"
        assert_eq!(issue_ref("2-123"), json!({"id": "2-123"}));
        // readable ids (project-prefixed) use idReadable
        assert_eq!(issue_ref("DEMO-1"), json!({"idReadable": "DEMO-1"}));
    }

    // ---- pull-request extraction ----

    #[test]
    fn pr_changes_from_activities_reads_state_id() {
        // shape mirrors a live PullRequestChangeCategory response: state on
        // `state.id` (name is null), wrapped in each item's `added` array.
        let acts = json!([
            {"$type": "PullRequestChangeActivityItem",
             "added": [{"$type": "PullRequestChange", "state": {"id": "OPEN"}}]},
            {"$type": "PullRequestChangeActivityItem",
             "added": [{"$type": "PullRequestChange", "state": {"id": "MERGED"}, "url": "https://gh/pr/1"}]},
        ]);
        let changes = pr_changes_from_activities(&acts);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0], ("OPEN".into(), None));
        assert_eq!(
            changes[1],
            ("MERGED".into(), Some("https://gh/pr/1".into()))
        );
    }

    #[test]
    fn pr_changes_from_activities_skips_stateless_and_non_array() {
        // an added entry without a state id is skipped; non-array inputs yield empty
        let acts = json!([{"added": [{"$type": "PullRequestChange"}]}]);
        assert!(pr_changes_from_activities(&acts).is_empty());
        assert!(pr_changes_from_activities(&json!([])).is_empty());
        assert!(pr_changes_from_activities(&json!(null)).is_empty());
    }

    #[test]
    fn has_merged_pr_matches_merged_case_insensitively() {
        let merged = vec![("merged".to_string(), None)];
        let open = vec![("OPEN".to_string(), None)];
        let declined = vec![("DECLINED".to_string(), None)];
        assert!(has_merged_pr(&merged));
        // not-merged states (incl. DECLINED) must not count
        assert!(!has_merged_pr(&open));
        assert!(!has_merged_pr(&declined));
        assert!(!has_merged_pr(&[]));
        // a PR that went OPEN -> MERGED still counts
        assert!(has_merged_pr(&[
            open[0].clone(),
            ("MERGED".to_string(), None)
        ]));
    }

    // ---- self-update helpers ----

    #[test]
    fn parse_version_handles_prefix_and_suffix() {
        assert_eq!(parse_version("v0.6.1"), Some((0, 6, 1)));
        assert_eq!(parse_version("0.6.1"), Some((0, 6, 1)));
        assert_eq!(parse_version("1.2.3-rc1"), Some((1, 2, 3)));
        assert_eq!(parse_version("2.0"), Some((2, 0, 0)));
        assert_eq!(parse_version("nope"), None);
    }

    #[test]
    fn is_newer_compares_semver() {
        assert!(is_newer("0.7.0", "0.6.1"));
        assert!(is_newer("0.6.2", "0.6.1"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.6.1", "0.6.1"));
        assert!(!is_newer("0.6.0", "0.6.1"));
        // v-prefix tolerated on either side
        assert!(is_newer("v0.7.0", "0.6.1"));
        // unparseable => not newer (fail safe)
        assert!(!is_newer("garbage", "0.6.1"));
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
