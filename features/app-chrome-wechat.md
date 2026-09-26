# Task: connect the `app` alias to Chrome WeChat (closed 2026-09-26)

Request: `app 接上 chrome wechat` — make the seeded `app` alias
(`@native app`, src/launch.rs) open WeChat through Chrome/Chromium.

## Machine facts (verified 2026-09-26)

- No native WeChat desktop entry installed; Chrome here is
  `chromium-browser` (`/usr/bin`) and `/snap/bin/chromium`; `google-chrome`
  is absent.
- `~/.local/share/applications` did not exist. `desktop::scan`
  (src/launch/desktop/scan.rs) reads it first once present — a user-level
  `.desktop` file needs no code change to be matched.
- Matching already covers `微信` (name), `wechat` (keyword) and `wx`
  (pinyin initials; the table in src/launch/pinyin_table.rs has 微/信).
- URL passthrough (`launch::url_fallback`) was macOS-only at `f9dcbbd`.

## Approved blueprint (if (re)implementation is ever needed)

- A, data wiring: `assets/wechat-web.desktop` template —
  `Exec=<browser> --app=https://wx.qq.com/`, `StartupWMClass=wx.qq.com`,
  `Icon=chromium-browser` — plus `scripts/xc-wechat` installer in
  `xc-icon` style (probe google-chrome → chromium → chromium-browser →
  /snap/bin/chromium, install into `$XDG_DATA_HOME/applications`, `-u`
  uninstall, `--prefix D`). Tradeoff: exec basename `chromium` makes
  `app chromium` ambiguous with the system browser entry (safe list, no
  guess-launch).
- B, optional code: port URL passthrough to Linux in `src/launch.rs`
  (`url_fallback` drop the macOS gate; `command_for` gains an
  `xdg-open` → `gio open` branch for `exec: None` targets); tests in
  `src/launch/tests.rs`.

## Verified state at close (blueprint applied 2026-09-26)

A + B are both in the tree now (README `app`/web-apps wording landed via
3dc9034; the rest in the two `feat(app)` commits that follow):

- `assets/wechat-web.desktop` + `scripts/xc-wechat` (xc-icon style;
  detected browser here: `/snap/bin/chromium`). Installed to
  `~/.local/share/applications/wechat-web.desktop`.
- `src/launch.rs`: `url_fallback` passes `://` inputs through on both
  platforms when nothing matched; `command_for` routes URL targets to
  `xdg-open` (else `gio open`) before the desktop-entry launchers.
  `src/launch/tests.rs` covers both purely (no PATH dependence).

Verified headless: `cargo fmt/clippy/test` clean (448 tests);
`--print-app 微信|wx|wechat` -> the wechat entry; `--print-app
https://wx.qq.com` -> URL passthrough (rc 0); `--print-app chromium`
-> the expected 2-way ambiguity; `xc-wechat -u` round-trips. The live
bar check (`app wx` -> chromium app-mode window) needs a real session
(no DISPLAY here).
