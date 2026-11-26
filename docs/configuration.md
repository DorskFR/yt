# Configuration

## Credentials

`yt write server auth URL [TOKEN] [NAME]` writes `~/.config/yt/config.json` (or `$XDG_CONFIG_HOME/yt/config.json`) with mode 600. With TOKEN omitted or `-`, the token is read from stdin: a hidden prompt on a terminal, the whole input otherwise. A token typed on the command line still works but lands in shell history and the process list, so `yt` warns on stderr.

```sh
yt write server auth https://youtrack.example.com              # hidden prompt
printf %s "$TOKEN" | yt write server auth https://youtrack.example.com -
```

## Multiple servers

Credentials are keyed by server name. The first server added becomes the default; with no name it is stored as `default`.

```sh
yt write server auth https://yt.example.com                 # name "default"
yt write server auth https://yt.acme.com - acme             # named server
yt read server ls                                           # * marks the default
yt write server default acme                                # change the default
yt --server acme read issue ls "project: ACME #Unresolved"  # one-off override
```

A legacy single-server `config.json` (`{"url","token"}`) is read as the `default` server.

## Client certificates (mTLS)

For a server behind a gateway that requires a TLS client certificate, add `client_cert` and `client_key` (paths to PEM files) to that server's entry in `config.json`:

```json
{"servers": {"home": {"url": "https://yt.example.com", "token": "perm-...",
                      "client_cert": "/path/to/client.pem", "client_key": "/path/to/client.key"}}}
```

`YT_CLIENT_CERT` and `YT_CLIENT_KEY` override them, each on its own; they also apply with `YOUTRACK_URL`/`YOUTRACK_API_TOKEN`. The certificate file may hold a chain; the key file's first private key is used (PKCS#8, PKCS#1 or SEC1). Both must be set or neither. The certificate is only sent to a server that asks for one, so servers without mTLS are unaffected. `yt write server auth` keeps the paths of an existing entry.

Server certificates are checked against the platform roots; on Linux, `SSL_CERT_FILE` or `SSL_CERT_DIR` point at a private CA. If the server asks for a certificate and none is configured, the error ends with a hint naming these settings.

## Resolution order

1. `YOUTRACK_URL` and `YOUTRACK_API_TOKEN`, when both are set and no `--server` is given. If only one is set, `yt` warns on stderr and ignores it.
2. `--server NAME`.
3. The configured default, or the only configured server.

## Environment variables

| Variable | Effect |
|---|---|
| `YOUTRACK_URL`, `YOUTRACK_API_TOKEN` | Server and token; used only as a pair |
| `YT_CLIENT_CERT`, `YT_CLIENT_KEY` | PEM client certificate and key paths; override the server's `client_cert`/`client_key` |
| `YT_AGENT=1` | Same as `--agent` |
| `YT_MUTE_NOTIFICATIONS=1` | Same as `--silent` |
| `YT_NO_UPDATE_CHECK=1` | Disable the update notice |
| `NO_COLOR`, `CLICOLOR`, `CLICOLOR_FORCE` | Colour control |
