# xconsoler

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
same key whenever you like.

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
| `clipboard` (`cd`)| `wl-copy @stdin \|\| xclip … \|\| xsel …`     | `pbcopy @stdin`   |

## Custom aliases (`:` commands)

Type a line starting with `:` and press Enter:

```
:add <name>[,<shortcut>...] <linux-cmd> // <macos-cmd>
:del <name>
:help
```

Examples:

```
:add gh,git-open xdg-open https://github.com/{input} // open https://github.com/{input}
:add clip-mac - // pbcopy @stdin      ('-' = not configured on linux)
:del gh
```

Placeholders:

* `{input}` — everything you typed after the alias, shell-quoted into the command.
* `@stdin` — your input is piped to the command's **stdin** instead.

Built-ins cannot be deleted; re-define them with `:add` to override.

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
