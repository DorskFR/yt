# yt — token-frugal YouTrack CLI

A tiny YouTrack CLI designed for coding agents: compact line-oriented output,
tight `fields=` requests so the server never sends `$type`/nested-object bloat,
and ID-only output on writes. A search of 20 issues is ~2 KB instead of ~47 KB,
creating an issue prints just the new ID, and a comment prints `ok`.

## Install

Download a prebuilt binary from the [latest release](https://github.com/DorskFR/yt/releases/latest)
(`yt-darwin-arm64`, `yt-darwin-amd64`, `yt-linux-amd64`, `yt-linux-arm64`):

```sh
curl -L https://github.com/DorskFR/yt/releases/latest/download/yt-darwin-arm64 -o yt
chmod +x yt && install -m755 yt ~/.local/bin/
```

Or build from source (needs a Rust toolchain):

```sh
cargo install --git https://github.com/DorskFR/yt
# or: cargo build --release && install -m755 target/release/yt ~/.local/bin/
```

## Setup

Save credentials once (written to `~/.config/yt/config.json`, mode 600):

```sh
yt auth https://youtrack.example.com perm-...   # token "-" reads stdin
```

### Multiple servers

Credentials are keyed by server name, so one config can hold several instances:

```sh
yt auth https://yt.example.com perm-...          # no name -> "default" (first added becomes default)
yt auth https://yt.acme.com perm-... acme        # named server
yt servers                                       # list servers (* marks the default)
yt default acme                                  # change the default
yt --server acme ls "project: ACME #Unresolved"  # one-off override for any command
```

Resolution order for every command: `YOUTRACK_URL`/`YOUTRACK_API_TOKEN` env vars
(when no `--server` is given) → `--server NAME` → the configured default. A
legacy single-server `config.json` is read transparently as the `default` server.

`YOUTRACK_URL` / `YOUTRACK_API_TOKEN` env vars override the config file when set.

## Usage

```
yt ls "QUERY" [-n 20] [--full]        search; one line per issue: ID  STATE  PRIO  SUMMARY
yt show ID [-c]                       issue detail; -c appends comments
yt new PROJECT "SUMMARY" [-d DESC|-d -] [-f "Priority Critical"]...   prints new ID only
yt comment ID [TEXT]                  add comment (stdin if TEXT omitted)
yt comments ID                        list comments
yt cmd "COMMAND" ID... [-m COMMENT]   apply command: state, assignee, tags, ...
yt projects                           list projects (SHORT  NAME)
yt fields PROJECT                     fields + allowed values (falls back to observed
                                      values when the token lacks project-admin rights)
yt me / yt users QUERY                user info
yt query-help                         query syntax cheat sheet
yt auth URL TOKEN [NAME]              save credentials (NAME defaults to "default")
yt servers                            list configured servers (* marks the default)
yt default NAME                       set the default server
--server NAME                         (global) use a named server for any command
```

`-f` on `new` uses YouTrack command syntax and is applied right after creation
(`-f "Priority Critical" -f "Type Bug"`).

### Examples

```sh
yt ls "project: DEMO #Unresolved sort by: updated desc" -n 10
yt show DEMO-42 -c
yt new DEMO "Login button misaligned" -d - -f "Priority Major"
yt cmd "State {In Progress} assignee me" DEMO-42 DEMO-43 -m "picking this up"
```

## Agent setup

Paste a snippet like this into your agent's `CLAUDE.md` (or equivalent) so it
uses `yt` instead of a heavier MCP integration:

```markdown
## YouTrack
Use the `yt` CLI for issue tracking (auth already configured):
- `yt ls "project: DEMO #Unresolved sort by: updated desc" [-n N] [--full]` — search
- `yt show DEMO-1 [-c]` — detail (+comments); `yt comments DEMO-1`
- `yt new DEMO "summary" -d - [-f "Priority Critical"]` — create, desc from stdin, prints ID
- `yt comment DEMO-1 "text"` — comment
- `yt cmd "State Done assignee me" DEMO-1 DEMO-2 [-m "note"]` — batch state/assign/etc.
- `yt projects`, `yt fields DEMO`, `yt query-help` — discovery
```

## License

MIT
</content>
</invoke>
