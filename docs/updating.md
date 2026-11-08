# Updating

```sh
yt write update            # download the latest release, verify sha256, replace in place
yt write update --force    # reinstall the latest even if already current
```

- Picks the prebuilt binary for the current OS and architecture (Linux, macOS; amd64, arm64).
- Checks its sha256 against the release `SHA256SUMS` before installing.
- Replaces the running binary with an atomic rename, so only the install directory must be writable.

## Update notice

When stderr is a terminal, `yt` prints a one-line `update available: …` notice when a newer release exists. The check hits GitHub at most once every 24 h, caches the result in `~/.config/yt/update-check.json`, and is silent on failure. `YT_NO_UPDATE_CHECK=1` or `--agent` disables it.
