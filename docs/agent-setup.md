# Agent setup

Snippet for an agent's `CLAUDE.md` (or equivalent) so it uses `yt` instead of an MCP integration:

```markdown
## YouTrack
Use the `yt` CLI for issue tracking (auth already configured). Commands live
under `yt read …` (safe) or `yt write …` (mutating):
- `yt read issue ls "project: DEMO #Unresolved sort by: updated desc" [-n N] [--skip N] [--full]` — search
- `yt read issue show DEMO-1 [-c] [--pr]` — detail (+comments/PRs); `yt read issue comments DEMO-1`
- `yt read issue open DEMO-1 [-p]` — print the web URL and open it in a browser (`-p` prints only)
- `yt write issue new DEMO "summary" -d - [-f "Priority Critical"]` — create, desc from stdin, prints ID
- `yt write issue edit DEMO-1 -s "new summary" -d -` — edit summary/description (desc from stdin)
- `yt write issue comment DEMO-1 "text" [--group Devs] [--user alice]` — comment (restricted visibility optional)
- `yt write issue comment-edit DEMO-1 4-123 "fixed text"` / `yt write issue comment-delete DEMO-1 4-123` — edit or remove a comment
- `yt write issue comment-visibility DEMO-1 4-123 --public` — change a comment's visibility
- `yt read issue links DEMO-1`, `yt write issue link DEMO-1 "relates to" DEMO-2`, `yt write issue unlink DEMO-1 "depends on" DEMO-3` — relations
- `yt read issue attachments DEMO-1 [-o DIR]` — list attachments; `-o` downloads them (default cwd)
- `yt write issue attach DEMO-1 shot.png log.txt [-c 4-9]` — upload files to the issue (or comment `4-9`)
- `yt write issue cmd "State Done assignee me" DEMO-1 DEMO-2 [-m "note"]` — batch state/assign/etc.
- `yt read issue tags` — list tags; `yt write issue tag DEMO-1 Blocked` / `yt write issue untag DEMO-1 Blocked` — add/remove tag
- `yt read project ls`, `yt read project fields DEMO`, `yt read query-help` — discovery
- `yt read --json issue show DEMO-1` — raw JSON with entity ids; `yt read api issues/DEMO-1/comments --fields id,text --all` — any GET (`--all` pages through collections)
- `--silent` on any write mutes watcher notifications
```
