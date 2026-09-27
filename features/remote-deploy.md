# Remote deploy over SSH (2026-09-27)

Released HEAD `8e8a732` (binary unchanged since `78577fa`; the last commits
are doc-only) to the known SSH target from `features/pw-plugin.md`:
`root@192.168.31.196` (Ubuntu 24.04, x86_64). `scripts/xc-deploy` smoke test
passed (`--print-rows` = 4); local/remote md5 match (`204b23e2…`).

## Gotchas for the next deploy

- `scp: Text file busy` — background `xconsoler --internal-clipboard-serve`
  helpers hold the installed binary. Free it with `pkill -x xconsoler`;
  the interactive bar itself was not running (xc-bar is a toggle, not a loop).
- Over ssh, `pkill -f "xconsoler …"` matches the *remote shell's own command
  line* and kills the ssh session (exit 255). Always `pkill -x` there.
- Re-summoning the bar for the desktop session user (`m`, `:0`) from a root
  ssh session needs `DISPLAY`, `XAUTHORITY` and the session's real
  `DBUS_SESSION_BUS_ADDRESS` (read it from `/proc/<session-pid>/environ`;
  here `unix:path=/run/user/1000/bus`) — without D-Bus, gnome-terminal
  spawns nothing and xc-bar fails silently. The summoned bar is dismissed
  by the session's alt+d as usual.
