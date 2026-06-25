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
yt edit ID [-s "SUMMARY"] [-d DESC|-d -]   edit summary/description; prints ID
yt comment ID [TEXT]                  add comment (stdin if TEXT omitted)
yt comments ID                        list comments
yt attachments ID [-o DIR]            list attachments (NAME SIZE); -o downloads to DIR (default .)
yt attach ID FILE... [-c COMMENT]     upload files to an issue (or a comment with -c); prints ID NAME
yt cmd "COMMAND" ID... [-m COMMENT]   apply command: state, assignee, tags, ...
yt tags                               list tags (one name per line)
yt tag ID TAG                         add a tag (by name) to an issue
yt untag ID TAG                       remove a tag (by name) from an issue
yt projects                           list projects (SHORT  NAME)
yt project new SHORT NAME              create a project (prints SHORT  ID); needs an admin token
yt fields PROJECT                     fields + allowed values (falls back to observed
                                      values when the token lacks project-admin rights)
yt me / yt users QUERY                user info
yt query-help                         query syntax cheat sheet
yt auth URL TOKEN [NAME]              save credentials (NAME defaults to "default")
yt servers                            list configured servers (* marks the default)
yt default NAME                       set the default server
yt completions SHELL                  print a completion script (bash|zsh|fish|powershell|elvish)
--server NAME                         (global) use a named server for any command
```

### Shell completions

Generate a completion script for your shell and source it:

```sh
yt completions bash > ~/.local/share/bash-completion/completions/yt
yt completions zsh  > "${fpath[1]}/_yt"
yt completions fish > ~/.config/fish/completions/yt.fish
```

### Color

`yt` mostly prints plain, line-oriented output; clap's own help/error messages
are colorized. Output is routed through [`anstream`](https://docs.rs/anstream),
so color is auto-disabled when stdout/stderr is not a TTY (e.g. piped to a file
or another program) and honors the `NO_COLOR` and `CLICOLOR`/`CLICOLOR_FORCE`
environment variables — agent-safe by default.

`-f` on `new` uses YouTrack command syntax and is applied right after creation
(`-f "Priority Critical" -f "Type Bug"`).

`yt project new` requires a token with **admin permissions** (it POSTs to
`/api/admin/projects`); a non-admin token returns `HTTP 403` and the server's
message is surfaced. The current user is set as project leader automatically.
Creating or attaching project custom fields is likewise admin-only and is not
implemented — use `yt fields PROJECT` to list a project's fields.

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
- `yt edit DEMO-1 -s "new summary" -d -` — edit summary/description (desc from stdin)
- `yt comment DEMO-1 "text"` — comment
- `yt attachments DEMO-1 [-o DIR]` — list attachments; `-o` downloads them (default cwd)
- `yt attach DEMO-1 shot.png log.txt [-c 4-9]` — upload files to the issue (or comment `4-9`)
- `yt cmd "State Done assignee me" DEMO-1 DEMO-2 [-m "note"]` — batch state/assign/etc.
- `yt tags` — list tags; `yt tag DEMO-1 Blocked` / `yt untag DEMO-1 Blocked` — add/remove tag
- `yt projects`, `yt fields DEMO`, `yt query-help` — discovery
```

## License

MIT
</content>
</invoke>
