# xconsoler

A long-bar keyboard launcher for your terminal/tmux. `xconsoler` runs as a
small TUI that stays **hidden** until you press **Alt+D**; type an alias, some
input, hit Enter — the mapped command runs, the bar hides nothing, the history
remembers. Built in Rust on [ratatui](https://ratatui.rs) + crossterm.

## Build & run

```sh
cargo build --release
./target/release/xconsoler                 # default store location
./target/release/xconsoler --store ~/x.json
xconsoler -h | --help                      # usage
```

## Keys

| Key (bar shown)        | Action                                  |
|------------------------|-----------------------------------------|
| `Alt+D`                | wake / hide the bar (works while hidden)|
| `Enter`                | run selected match / submit `:` command |
| `Esc`                  | hide the bar                            |
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
press `Alt+D` in the bar to hide it, `Ctrl+C` to quit. Without tmux, just run
`xconsoler` in a spare terminal/tab and use `Alt+D` to toggle the bar.
