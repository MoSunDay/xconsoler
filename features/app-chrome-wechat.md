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

## Verified state at close

User marked the task complete. Inspection of the repo at `f9dcbbd`
(clean tree, reflog, src/scripts/assets, `~/.local/share/applications`)
found **no artifacts of this feature** — no `xc-wechat`, no
`wechat-web.desktop`, no `url_fallback` change. Treat the blueprint above
as not yet applied in this repository.

Verify with:

```sh
xconsoler --print-app 微信   # prints the wechat-web entry once installed
xconsoler --print-app wx
cargo test                   # after any src/launch.rs change
```
