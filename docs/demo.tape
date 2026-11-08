# Render from the repo root against the local mock in docs/demo/:
#   CC_x86_64_unknown_linux_musl=gcc cargo build --release --target x86_64-unknown-linux-musl
#   docker run --rm -v "$PWD":/vhs ghcr.io/charmbracelet/vhs docs/demo.tape
Output docs/assets/demo.gif

Set Shell bash
Set FontSize 18
Set Width 1320
Set Height 680
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 35ms
Set Framerate 20

Hide
Type `unset XDG_CONFIG_HOME $(compgen -e | grep -E '^(YOUTRACK|YT)_'); export HOME=$(mktemp -d) PATH=/vhs/target/x86_64-unknown-linux-musl/release:$PATH YOUTRACK_URL=http://127.0.0.1:18431 YOUTRACK_API_TOKEN=perm-demo YT_NO_UPDATE_CHECK=1 PS1='$ '`
Enter
Type "python3 -I /vhs/docs/demo/mock_youtrack.py 18431 2>/dev/null & sleep 1; clear"
Enter
Sleep 1500ms
Show

Type `yt read issue ls "project: DEMO #Unresolved"`
Sleep 300ms
Enter
Sleep 2s

Type "yt read issue show DEMO-42 -c"
Sleep 300ms
Enter
Sleep 3s

Type `yt write issue new DEMO "Retry button ignores rate-limit headers"`
Sleep 300ms
Enter
Sleep 1500ms

Type `yt write issue comment DEMO-58 "Also happens on the checkout page."`
Sleep 300ms
Enter
Sleep 4s
