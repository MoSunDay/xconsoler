<p align="center">
  <img src="assets/logo.svg" alt="xconsoler" width="520">
</p>

<p align="center">
  <em>A long-bar keyboard launcher for your terminal.</em>
</p>

A long-bar keyboard launcher for your terminal/tmux. `xconsoler` runs as a
small TUI that starts **shown** — type an alias, some input, hit Enter — the
mapped command runs and the history remembers. Press **alt+d** to hide the
bar, **alt+d** again to wake it. Built in Rust on
[ratatui](https://ratatui.rs) + crossterm.

## Build & run

```sh
cargo build --release
sudo cp target/release/xconsoler /usr/local/bin/
xconsoler                                  # default store location
xconsoler --store ~/x.json
xconsoler --summon                         # shell-keybind mode (see SSH section)
xconsoler --print-bind [--shell zsh]       # emit the shell binding line
xconsoler --set-wake-key alt+j             # change + persist the wake key
xconsoler -h | --help                      # usage
```

The bar is **visible on startup** — a fresh launch never looks like it
exited immediately. Hide it with the wake key and bring it back with the
same key whenever you like. Its terminal title is set to `xconsoler` (and
cleared again on exit, so your shell prompt can re-apply its own).

## Keys

| Key (bar shown)        | Action                                  |
|------------------------|-----------------------------------------|
| `alt+d` (configurable) | wake / hide the bar (works while hidden)|
| `Enter`                | run selected match / submit `:` command |
| `Esc`                  | hide the bar (quits in `--summon` mode) |
| `↑` `↓` / `Tab`        | move selection                          |
| `Backspace`            | delete last char                       |
| `Ctrl+U`               | clear the input                        |
| `Ctrl+C`               | quit (any state)                       |

## Built-in aliases

| Alias (shortcut)  | Linux                                        | macOS             |
|-------------------|----------------------------------------------|-------------------|
| `browser` (`br`)  | `xdg-open {input} >/dev/null 2>&1`           | `open {input}`    |
| `clipboard` (`cd`)| `@native clipboard` (built-in Rust backend)    | `@native clipboard` |

## Custom aliases (`:` commands)

Type a line starting with `:` and press Enter:

```
:add <name>[,<shortcut>...] <linux-cmd> // <macos-cmd>
:del <name>
:arg <name> <key> <value...>       # register a named argument
:unarg <name> <key>                # remove a named argument
:help
```

Examples:

```
:add gh,git-open xdg-open https://github.com/{input} // open https://github.com/{input}

The `clipboard` alias copies through the [`arboard`](https://crates.io/crates/arboard)
crate directly — no `xclip` / `wl-copy` / `xsel` / `pbcopy` binaries required
(Linux needs X11 or XWayland). Override it with `:add clipboard <cmd> // <cmd>`
if you prefer your own tool.
:add clip-mac - // pbcopy @stdin      ('-' = not configured on linux)
:del gh
```

Placeholders:

* `{input}` — everything you typed after the alias, shell-quoted into the command.
* `@stdin` — your input is piped to the command's **stdin** instead.

Built-ins cannot be deleted; re-define them with `:add` to override.

### Named arguments

Give an alias short names for the inputs you use all the time:

```
:arg br baidu https://www.baidu.com
```

Now `br baidu` opens `https://www.baidu.com`, while plain `br` and
`br <anything else>` behave exactly as before. As soon as you type
`<alias> `, the named args show up as sub-candidates (`↳ baidu ·
https://www.baidu.com`) — Enter on one runs its value. Values may contain
spaces; history keeps the raw text (`baidu`), so the shorthand stays
replayable. Setting an argument on a built-in alias materializes it as an
overriding user definition.

## Settings page (`/settings`)

Type `/settings` and press Enter for a full-screen alias manager
(`Esc` or `q` returns to the bar):

* `↑`/`↓` (or `j`/`k`) move the selection, `Enter`/`→` expands an alias to
  show its named args, `←` collapses.
* `n` — wizard for a new alias: name → shortcuts (comma-separated, may be
  empty) → linux command (use `{input}` where the input goes) → macos
  command (empty = same as linux).
* `a` — wizard for a new named argument on the selected alias: key → value.
* `d` — delete the selection: an arg row drops that argument, an alias row
  drops the whole alias (built-ins refuse; override them instead).

Every change is saved to `store.json` immediately.

## Storage

* Store lives at `<config_dir>/xconsoler/store.json` (override with `--store`).
* Config (currently just `wake_key`, default `alt+d`) is persisted in the
  same file; stores written before it existed load with the default.
  Change it with `xconsoler --set-wake-key <spec>`.
* Inputs are stored as **base64 of the plain text**, so quotes/unicode/newlines
  round-trip safely and nothing secret-looking is kept in cleartext.
* History: up to **10 000** entries, newest first, deduplicated per
  `alias + input` (re-running moves the entry to the top).
* Matching: fuzzy over `label + input` (history) and `name + shortcuts`
  (aliases); empty input lists recent history first, then aliases.

## Global wake-up

True global binding needs a multiplexer. With tmux (`.tmux.conf`):

```
bind -n M-d run -b 'xconsoler'
```

`run -b` keeps tmux responsive while the launcher runs in a background pane;
press `alt+d` in the bar to hide it, `Ctrl+C` to quit. Without tmux, use the
SSH summon flow below, or just run `xconsoler` in a spare terminal/tab and
use `alt+d` to toggle the bar.

## Desktop global hotkey (X11 / XFCE)

On a real desktop, register a **system-wide** shortcut so the long bar pops
up anywhere — no terminal, no tmux. `scripts/xc-bar` toggles the bar in its
own window; `scripts/xc-key` registers/re-binds the XFCE hotkey:

```sh
sudo install -m 0755 scripts/xc-bar scripts/xc-key /usr/local/bin/

# register alt+d -> xc-bar in the running XFCE session (applied live):
xc-key alt+d        # or, say: xc-key ctrl+g

# icon set + menu entry for the summoned window (~/.local/share by default;
# add --system for /usr/local/share):
scripts/xc-icon
```

### Logo & icon

`assets/icon.svg` is the mark (the launcher bar drawn as an icon), `assets/logo.svg`
the mark + wordmark lockup, and `assets/png/xconsoler-<size>.png` the raster icon
set. All of it is generated — one geometry spec, SVG and raster backends, so they
can never drift:

```sh
python3 scripts/gen-logo.py              # writes assets/**
python3 scripts/gen-logo.py --preview    # ASCII proof, writes nothing
scripts/xc-icon                          # install icons + xconsoler.desktop
scripts/xc-icon --uninstall              # remove them again
```

The palette is `src/theme.rs` (kanagawa wave): accent `#7e9cd8` for the chevron,
cursor `#c8c093` for the block cursor, `#2d4f67` for the selected candidate row.
Changing the theme means changing those consts in `scripts/gen-logo.py` and
re-running it. `xc-icon` installs each size twice — as `xconsoler` (menu entry)
and as `XConsoler` (the WM_CLASS `xc-bar` gives the summoned window, which is how
the taskbar finds the icon). Regenerating needs Pillow (`pip install pillow`).

`xc-bar` spawns a `gnome-terminal` window (`--class=XConsoler`, 140x14 near
the top) with a dedicated opaque GNOME Terminal profile over the pinned
kanagawa palette (`use-transparent-background=false`, `background-color=#1f1f28`,
`foreground-color=#dcdcdc`, `use-theme-colors=false`) because the launcher
paints no backgrounds of its own. Profile setup is idempotent and falls back
to a default-coloured window (one-line stderr warning) when gsettings/dconf
are unavailable. It then runs `xconsoler --summon` inside that window. Calling it again while the bar is up kills
the bar instead of opening a second one, and a successful run dismisses it
automatically, Spotlight-style.

## SSH deployment (no tmux)

Over plain SSH you can bind a shell key at the prompt so the launcher
"summons" like a global hotkey:

```sh
cargo build --release
sudo cp target/release/xconsoler /usr/local/bin/

# print the binding for your rc file (~/.bashrc):
xconsoler --print-bind                 # >> paste into ~/.bashrc
xconsoler --print-bind --shell zsh     # zsh flavour (bindkey + ZLE widget)
```

The default bash binding (one line, paste it into `~/.bashrc`):

```sh
bind -x '"\ed": "xconsoler --summon"' 2>/dev/null || true
```

`--summon` semantics:

* press the bound key **at the shell prompt** → the launcher pops up
  (it starts shown, never "opens and instantly exits");
* press the **wake key again or `Esc`** → it exits and you are back at the
  prompt, exactly where you were;
* the launcher runs *inside* the keybinding, so it owns the terminal until
  it exits — no tmux required.

Split-ESC protection: some terminals/pties deliver `Alt+D` as two events
(`Esc`, then `d`). The event loop holds a lone `Esc` for 40 ms and merges a
following plain character into `Alt+<char>`, so the wake key keeps working.

### Changing the wake key

```sh
xconsoler --set-wake-key alt+j     # validates, persists, prints a reminder
xconsoler --print-bind             # re-generate the binding for the new key
```

Then replace the old `bind`/`bindkey` line in your rc file and reload it.
A per-run override (not persisted) is also available:
`xconsoler --summon --wake-key ctrl+g`.

Note: binding `M-d` overrides readline's `M-d` (`kill-word`) at the prompt —
pick another key (`alt+j`, `ctrl+g`, …) if you use `M-d` while editing.
