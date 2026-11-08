# Configuration

## Credentials

`yt write server auth URL TOKEN [NAME]` writes `~/.config/yt/config.json` (or `$XDG_CONFIG_HOME/yt/config.json`) with mode 600. A token of `-` is read from stdin.

```sh
yt write server auth https://youtrack.example.com perm-...
```

## Multiple servers

Credentials are keyed by server name. The first server added becomes the default; with no name it is stored as `default`.

```sh
yt write server auth https://yt.example.com perm-...        # name "default"
yt write server auth https://yt.acme.com perm-... acme      # named server
yt read server ls                                           # * marks the default
yt write server default acme                                # change the default
yt --server acme read issue ls "project: ACME #Unresolved"  # one-off override
```

A legacy single-server `config.json` (`{"url","token"}`) is read as the `default` server.

## Resolution order

1. `YOUTRACK_URL` and `YOUTRACK_API_TOKEN`, when both are set and no `--server` is given. If only one is set, `yt` warns on stderr and ignores it.
2. `--server NAME`.
3. The configured default, or the only configured server.

## Environment variables

| Variable | Effect |
|---|---|
| `YOUTRACK_URL`, `YOUTRACK_API_TOKEN` | Server and token; used only as a pair |
| `YT_AGENT=1` | Same as `--agent` |
| `YT_MUTE_NOTIFICATIONS=1` | Same as `--silent` |
| `YT_NO_UPDATE_CHECK=1` | Disable the update notice |
| `NO_COLOR`, `CLICOLOR`, `CLICOLOR_FORCE` | Colour control |
