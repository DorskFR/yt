# yt

YouTrack CLI for coding agents: one line per issue on reads, only the ID on writes.

[![release](https://img.shields.io/github/v/release/DorskFR/yt)](https://github.com/DorskFR/yt/releases/latest)
[![CI](https://github.com/DorskFR/yt/actions/workflows/ci.yml/badge.svg)](https://github.com/DorskFR/yt/actions/workflows/ci.yml)
[![license](https://img.shields.io/github/license/DorskFR/yt)](LICENSE)

![yt listing, showing, creating and commenting on issues in a demo project](docs/assets/demo.gif)

Requests only the fields it prints: a 20-issue search returns about 2 KB instead of about 47 KB of API JSON. Commands split into `yt read` and `yt write`, so an access guard can allow or block each tier by prefix.

```sh
yt read issue ls "project: DEMO #Unresolved"         # one line per issue
yt read issue show DEMO-42 -c                        # fields, links, comments
yt write issue new DEMO "Summary" -d -               # prints the new ID; description from stdin
yt write issue comment DEMO-42 "text"                # prints ok
yt write issue cmd "State {In Progress}" DEMO-42     # any YouTrack command, on one or more issues
yt read --json issue show DEMO-42                    # raw JSON with entity ids
```

## Install

Prebuilt binaries are on the [latest release](https://github.com/DorskFR/yt/releases/latest): `yt-darwin-arm64`, `yt-darwin-amd64`, `yt-linux-amd64`, `yt-linux-arm64`.

```sh
curl -L https://github.com/DorskFR/yt/releases/latest/download/yt-darwin-arm64 -o yt
chmod +x yt && install -m755 yt ~/.local/bin/
```

From source (Rust toolchain):

```sh
cargo install --git https://github.com/DorskFR/yt
# or: cargo build --release && install -m755 target/release/yt ~/.local/bin/
```

Update with `yt write update`. Details in [docs/updating.md](docs/updating.md).

## Quick start

```console
$ yt write server auth https://youtrack.example.com perm-...   # token "-" reads stdin
$ yt read issue ls "project: DEMO #Unresolved"
DEMO-42  In Progress  Critical  @alice  Login page times out on slow networks #auth
DEMO-47  Open         Major     @bob    Password reset email has a broken link #auth #email
DEMO-51  Open         Normal    -       Export to CSV drops the last row #export
```

## Configuration

Credentials are stored in `~/.config/yt/config.json` (mode 600), keyed by server name.

| Setting | Effect |
|---|---|
| `YOUTRACK_URL` + `YOUTRACK_API_TOKEN` | Override the configured server; used only as a pair |
| `--server NAME` | Use a named server from the config |
| `--agent` / `YT_AGENT=1` | No colour, no update notice |
| `--silent` / `YT_MUTE_NOTIFICATIONS=1` | Mute watcher notifications on writes |
| `YT_NO_UPDATE_CHECK=1` | Disable the update notice |

- Multiple servers and resolution order: [docs/configuration.md](docs/configuration.md)
- Every command, JSON output, comments, completions: [docs/reference.md](docs/reference.md) and `yt --help`
- `CLAUDE.md` snippet for agents: [docs/agent-setup.md](docs/agent-setup.md)

## Limitations

- No Windows binary.
- Project custom fields cannot be created or attached (admin-only API, not implemented).
- `yt write project create` needs an admin token.
- `--silent` takes effect only with the *Apply Commands Silently* permission.
- `comment-delete --permanent` needs the *Delete Comment permanently* permission; without it the comment is only trashed.

## Development

```sh
git clone https://github.com/DorskFR/yt && cd yt
cargo test
```

The demo runs against a local mock server ([docs/demo/mock_youtrack.py](docs/demo/mock_youtrack.py)); render steps are in [docs/demo.tape](docs/demo.tape).

## License

[MIT](LICENSE)
