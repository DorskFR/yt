# Command reference

`yt --help` and `yt <tier> <noun> --help` print the same list with every flag.

The tree is `tier → noun → verb`. `yt read …` never mutates anything; `yt write …` changes issues, projects, local config or the binary. An access guard can gate the whole CLI on these two prefixes.

## Read tier

```
yt read issue ls "QUERY" [-n 20] [--skip N] [--full] [--merged-pr]   search; one line per issue: ID  STATE  PRIO  @ASSIGNEE  SUMMARY [#tag ...]
                                            (a `# limit N reached; next page: --skip M` hint goes to stderr when -n is hit;
                                            --merged-pr keeps only issues with a merged PR, one extra call per row)
yt read issue show ID [-c] [--pr]           issue detail (fields, tags, links); -c appends comments, --pr linked PRs
yt read issue open ID [-p]                  print ID  SUMMARY + web URL, then open it in the browser
                                            (-p/--print-only, --json, or a non-TTY stdout skip the launch)
yt read issue comments ID                   list all comments (comments, links, attachments, tags
                                            and projects are always fetched in full, page by page)
yt read issue links ID                      list links (PHRASE  ID  SUMMARY), grouped by relation
yt read issue attachments ID [-o DIR]       list attachments (NAME SIZE); -o downloads to DIR (default .)
yt read issue tags                          list tags (one name per line)
yt read project ls [--all]                  list projects (SHORT  NAME); --all includes archived
yt read project fields PROJECT              fields + allowed values (falls back to observed
                                            values when the token lacks project-admin rights)
yt read user me                             authenticated user
yt read user ls QUERY [-n N] [--skip N]     search users by name/login (default 10)
yt read server ls                           list configured servers (* marks the default)
yt read query-help                          query syntax cheat sheet
yt read api PATH [--fields F] [--query K=V]... [--top N] [--skip N] [--all]   raw authenticated GET on /api/PATH; prints JSON
                                            (YouTrack returns 42 items without --top; --all fetches every page,
                                            and a stderr hint flags a response that is exactly one full page)
```

## Write tier

```
yt write issue new PROJECT "SUMMARY" [-d DESC|-d -] [-f "Priority Critical"]...  prints new ID only
yt write issue edit ID [-s "SUMMARY"] [-d DESC|-d -]   edit summary/description; prints ID
yt write issue comment ID [TEXT] [--public|--group G...|--user U...]   add comment (stdin if TEXT omitted); prints ok
yt write issue comment-edit ID COMMENT_ID [TEXT]   replace a comment's text (stdin if TEXT omitted)
yt write issue comment-delete ID COMMENT_ID [--permanent]   delete a comment (trash; --permanent erases it)
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
```

## Global flags

```
yt completions SHELL      print a completion script (bash|zsh|fish|powershell|elvish)
--server NAME             use a named server for any command
--silent                  mute watcher notifications on writes (or YT_MUTE_NOTIFICATIONS=1)
--agent                   plain, stable output: no colour, no update notice (or YT_AGENT=1)
--json                    (read tier, anywhere after `read`) raw JSON instead of text, with entity ids
```

## JSON output

`yt read --json …` prints the raw API response, so results carry entity `id`s that can feed `yt write` (for example comment ids for `comment-visibility`). Hints such as `# limit N reached` still go to stderr, so stdout stays valid JSON. `yt read api` always prints JSON.

| Command | `--json` shape |
|---|---|
| `issue show` | issue object plus a `links` array; `comments` with `-c`; `pullRequests` (`[{"state","url"}]`) with `--pr` |
| `server ls` | `[{"name","url","default"}]`, never the token |
| `query-help` | `{"text": ...}` |

## Colour and agent mode

On a terminal, text output is coloured: bold cyan issue ids, green or yellow states (dim once resolved), red priorities for critical and major, dim for minor, a magenta assignee, blue tags, dimmed metadata keys in `issue show`. `issue ls` pads the id, state, priority and assignee columns so summaries line up. clap help and errors are coloured too.

Output goes through [`anstream`](https://docs.rs/anstream): colour is off when stdout or stderr is not a TTY, and `NO_COLOR`, `CLICOLOR` and `CLICOLOR_FORCE` are honoured.

`--agent` (or `YT_AGENT=1`; the flag wins) forces colour off regardless of TTY and suppresses the `update available:` notice, as with `YT_NO_UPDATE_CHECK=1`. Line formats are unchanged, so the same greps work either way. It is the switch a harness sets once for a whole session.

## Silent writes

`--silent` (or `YT_MUTE_NOTIFICATIONS=1`; the flag wins) sends `muteUpdateNotifications=true` on `yt write issue …` calls so watchers are not emailed. The server honours it only when the token's user has the *Apply Commands Silently* permission.

## Comments

- Comment ids are the values in parentheses in `yt read issue comments` output (e.g. `4-123`).
- `comment` and `comment-visibility` accept `--public`, or any mix of `--group NAME` and `--user LOGIN` (both repeatable). Without them, `comment` uses the default visibility.
- `comment-edit ID COMMENT_ID [TEXT]` replaces the text, reading stdin when `TEXT` is omitted.
- `comment-delete ID COMMENT_ID` moves the comment to the trash, as the web UI does; an ordinary token may do this. Trashed comments disappear from `yt read issue comments` and carry `"deleted": true` under `--json`.
- `--permanent` also erases the comment. It needs the *Delete Comment permanently* permission; without it the comment is still trashed and the command exits non-zero.

## Fields on create

`-f` on `write issue new` takes YouTrack command syntax (`-f "Priority Critical" -f "Type Bug"`). Fields whose type can be resolved are set atomically in the create call. Otherwise (unknown field type, `Assignee me`, ...) they are applied as a command right after creation. `yt read project fields PROJECT` lists a project's fields and values.

## Projects

`yt write project create` POSTs to `/api/admin/projects` and needs a token with admin permissions; a non-admin token returns `HTTP 403` with the server's message. The current user becomes project leader. Creating or attaching project custom fields is also admin-only and is not implemented.

## Shell completions

`yt completions <shell>` prints a completion script to stdout. Re-run it after upgrading to pick up new commands.

```fish
mkdir -p ~/.config/fish/completions
yt completions fish > ~/.config/fish/completions/yt.fish
```

```bash
mkdir -p ~/.local/share/bash-completion/completions
yt completions bash > ~/.local/share/bash-completion/completions/yt
```

```zsh
yt completions zsh > "${fpath[1]}/_yt"   # any dir on $fpath
```

New shells load it automatically. To load it in the current shell, source the file (e.g. `source ~/.config/fish/completions/yt.fish`).
