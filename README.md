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

## Updating

```sh
yt write update            # download the latest release, verify sha256, replace in place
yt write update --force    # reinstall the latest even if already current
```

`yt write update` picks the right prebuilt binary for your OS/arch (Linux and macOS,
amd64/arm64), checks its sha256 against the release `SHA256SUMS` before
installing, and replaces the running binary via an atomic rename (so the install
dir just needs to be writable).

When stderr is a terminal, `yt` also prints a one-line `update available: …`
notice on a newer release. The check hits GitHub at most once every 24h (result
cached in `~/.config/yt/update-check.json`) and is silent on failure. Set
`YT_NO_UPDATE_CHECK=1` to disable it.

The command tree is split into two permission tiers — `yt read …` (never
mutates anything) and `yt write …` (issues, projects, local config, self-update)
— so an access guard can gate the whole CLI on two stable prefixes. Each tier is
`tier → noun → verb`, e.g. `yt read issue ls`, `yt write issue comment`.

## Setup

Save credentials once (written to `~/.config/yt/config.json`, mode 600):

```sh
yt write server auth https://youtrack.example.com perm-...   # token "-" reads stdin
```

### Multiple servers

Credentials are keyed by server name, so one config can hold several instances:

```sh
yt write server auth https://yt.example.com perm-...      # no name -> "default" (first added becomes default)
yt write server auth https://yt.acme.com perm-... acme    # named server
yt read server ls                                         # list servers (* marks the default)
yt write server default acme                              # change the default
yt --server acme read issue ls "project: ACME #Unresolved"  # one-off override for any command
```

Resolution order for every command: `YOUTRACK_URL`/`YOUTRACK_API_TOKEN` env vars
(when no `--server` is given) → `--server NAME` → the configured default. A
legacy single-server `config.json` is read transparently as the `default` server.

## Usage

```
# read tier — never mutates
yt read issue ls "QUERY" [-n 20] [--full] [--merged-pr]   search; one line per issue: ID  STATE  PRIO  SUMMARY [#tag ...]
                                            (a `# limit N reached` hint goes to stderr when -n is hit;
                                            --merged-pr keeps only issues with a merged PR, one extra call per row)
yt read issue show ID [-c] [--pr]           issue detail (fields, tags, links); -c appends comments, --pr linked PRs
yt read issue open ID [-p]                  print ID  SUMMARY + web URL, then open it in the browser
                                            (-p/--print-only, --json, or a non-TTY stdout skip the launch)
yt read issue comments ID                   list comments
yt read issue links ID                      list links (PHRASE  ID  SUMMARY), grouped by relation
yt read issue attachments ID [-o DIR]       list attachments (NAME SIZE); -o downloads to DIR (default .)
yt read issue tags                          list tags (one name per line)
yt read project ls [--all]                  list projects (SHORT  NAME); --all includes archived
yt read project fields PROJECT              fields + allowed values (falls back to observed
                                            values when the token lacks project-admin rights)
yt read user me                             authenticated user
yt read user ls QUERY [-n N]                search users by name/login (default 10)
yt read server ls                           list configured servers (* marks the default)
yt read query-help                          query syntax cheat sheet
yt read api PATH [--fields F] [--query K=V]... [--top N] [--skip N]   raw authenticated GET on /api/PATH; prints JSON

# write tier — mutates issues / projects / local config / the binary
yt write issue new PROJECT "SUMMARY" [-d DESC|-d -] [-f "Priority Critical"]...  prints new ID only
yt write issue edit ID [-s "SUMMARY"] [-d DESC|-d -]   edit summary/description; prints ID
yt write issue comment ID [TEXT] [--public|--group G...|--user U...]   add comment (stdin if TEXT omitted); default visibility unless restricted
yt write issue comment-visibility ID COMMENT_ID (--public|--group G...|--user U...)   change a comment's visibility; prints the result
yt write issue link ID "PHRASE" TARGET      link two issues, e.g. yt write issue link YT-1 "relates to" YT-2
yt write issue unlink ID "PHRASE" TARGET    remove a link (same phrase)
yt write issue attach ID FILE... [-c COMMENT]  upload files to an issue (or a comment with -c); prints ID NAME
yt write issue cmd "COMMAND" ID... [-m COMMENT]  apply command: state, assignee, tags, ...
yt write issue tag ID TAG                   add a tag (by name) to an issue
yt write issue untag ID TAG                 remove a tag (by name) from an issue
yt write project create SHORT NAME          create a project (prints SHORT  ID); needs an admin token
yt write server auth URL TOKEN [NAME]       save credentials (NAME defaults to "default")
yt write server default NAME                set the default server
yt write update [--force]                   self-update to the latest release

yt completions SHELL                        print a completion script (bash|zsh|fish|powershell|elvish)
--server NAME                               (global) use a named server for any command
--silent                                    (global) mute watcher notifications on writes (or YT_MUTE_NOTIFICATIONS=1)
--json                                      (read tier) raw JSON instead of text, with entity ids
```

### JSON output

`yt read --json …` (the flag goes anywhere after `read`) prints the raw API
response so results carry entity `id`s that can feed `yt write` (e.g. comment
ids for `comment-visibility`). Hints such as `# limit N reached` still go to
stderr, so stdout stays valid JSON. `yt read api` always prints JSON.

With `--json`, `issue show` emits the issue object plus a `links` array, and adds
`comments` (with `-c`) and `pullRequests` (`[{"state","url"}]`, with `--pr`).
`server ls` emits `[{"name","url","default"}]` (never the token) and `query-help`
emits `{"text": ...}`.

### Silent writes

`--silent` (or `YT_MUTE_NOTIFICATIONS=1`; the flag wins) sends
`muteUpdateNotifications=true` on `yt write issue …` calls so watchers are not
emailed. The server honors it only when the token's user has the *Apply
Commands Silently* permission.

### Comment visibility

`yt write issue comment` and `yt write issue comment-visibility` accept
`--public`, or any mix of `--group NAME` / `--user LOGIN` (both repeatable).
Comment ids are the values in parentheses in `yt read issue comments` output
(e.g. `4-123`).

### Shell completions

`yt completions <shell>` prints a completion script to stdout. Drop it in the
right place for your shell — copy-paste one block:

**fish**
```fish
mkdir -p ~/.config/fish/completions
yt completions fish > ~/.config/fish/completions/yt.fish
```

**bash**
```bash
mkdir -p ~/.local/share/bash-completion/completions
yt completions bash > ~/.local/share/bash-completion/completions/yt
```

**zsh** (any dir on your `$fpath`)
```zsh
yt completions zsh > "${fpath[1]}/_yt"
```

New shells pick it up automatically. To use it in the **current** shell without
opening a new one, source the file (e.g. `source ~/.config/fish/completions/yt.fish`).
Re-run the command after upgrading `yt` to refresh completions for new commands.

### Color

`read issue ls` colorizes issue IDs, State (green = resolved, yellow = active), and
Priority (red = critical/major, dim = minor); `read issue show` highlights the issue ID.
clap's help/error messages are colorized too. Output is routed through
[`anstream`](https://docs.rs/anstream), so color is auto-disabled when
stdout/stderr is not a TTY (e.g. piped to a file or another program) and honors
the `NO_COLOR` and `CLICOLOR`/`CLICOLOR_FORCE` environment variables — agent-safe
by default.

`-f` on `write issue new` uses YouTrack command syntax and is applied right after
creation (`-f "Priority Critical" -f "Type Bug"`).

`yt write project create` requires a token with **admin permissions** (it POSTs to
`/api/admin/projects`); a non-admin token returns `HTTP 403` and the server's
message is surfaced. The current user is set as project leader automatically.
Creating or attaching project custom fields is likewise admin-only and is not
implemented — use `yt read project fields PROJECT` to list a project's fields.

### Examples

```sh
yt read issue ls "project: DEMO #Unresolved sort by: updated desc" -n 10
yt read issue show DEMO-42 -c
yt write issue new DEMO "Login button misaligned" -d - -f "Priority Major"
yt write issue cmd "State {In Progress} assignee me" DEMO-42 DEMO-43 -m "picking this up"
```

## Agent setup

Paste a snippet like this into your agent's `CLAUDE.md` (or equivalent) so it
uses `yt` instead of a heavier MCP integration:

```markdown
## YouTrack
Use the `yt` CLI for issue tracking (auth already configured). Commands live
under `yt read …` (safe) or `yt write …` (mutating):
- `yt read issue ls "project: DEMO #Unresolved sort by: updated desc" [-n N] [--full]` — search
- `yt read issue show DEMO-1 [-c] [--pr]` — detail (+comments/PRs); `yt read issue comments DEMO-1`
- `yt read issue open DEMO-1 [-p]` — print the web URL and open it in a browser (`-p` prints only)
- `yt write issue new DEMO "summary" -d - [-f "Priority Critical"]` — create, desc from stdin, prints ID
- `yt write issue edit DEMO-1 -s "new summary" -d -` — edit summary/description (desc from stdin)
- `yt write issue comment DEMO-1 "text" [--group Devs] [--user alice]` — comment (restricted visibility optional)
- `yt write issue comment-visibility DEMO-1 4-123 --public` — change a comment's visibility
- `yt read issue links DEMO-1`, `yt write issue link DEMO-1 "relates to" DEMO-2`, `yt write issue unlink DEMO-1 "depends on" DEMO-3` — relations
- `yt read issue attachments DEMO-1 [-o DIR]` — list attachments; `-o` downloads them (default cwd)
- `yt write issue attach DEMO-1 shot.png log.txt [-c 4-9]` — upload files to the issue (or comment `4-9`)
- `yt write issue cmd "State Done assignee me" DEMO-1 DEMO-2 [-m "note"]` — batch state/assign/etc.
- `yt read issue tags` — list tags; `yt write issue tag DEMO-1 Blocked` / `yt write issue untag DEMO-1 Blocked` — add/remove tag
- `yt read project ls`, `yt read project fields DEMO`, `yt read query-help` — discovery
- `yt read --json issue show DEMO-1` — raw JSON with entity ids; `yt read api issues/DEMO-1/comments --fields id,text` — any GET
- `--silent` on any write mutes watcher notifications
```

## License

MIT
</content>
</invoke>
