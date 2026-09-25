<p align="center">
  <img src="assets/logo.svg" alt="xconsoler" width="520">
</p>

<p align="center">
  <em>A long-bar keyboard launcher for your terminal.</em>
</p>

A long-bar keyboard launcher for your terminal/tmux. `xconsoler` runs as a
small TUI that starts **shown** — type an alias, some input, hit Enter — the
mapped command runs and a run that works is remembered. Press **alt+d** for the
command palette, **Esc** to hide the bar, **alt+d** again to wake it. Built
in Rust on [ratatui](https://ratatui.rs) + crossterm.

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
exited immediately. Hide it with `Esc` and bring it back with the wake key
whenever you like. Its terminal title is set to `xconsoler` (and cleared
again on exit, so your shell prompt can re-apply its own).

## Keys

| Key (bar shown)        | Action                                  |
|------------------------|-----------------------------------------|
| `alt+d` (configurable) | open the command palette while shown; wake the bar while hidden |
| `Enter`                | run selected match / submit `:` command |
| `Esc`                  | hide the bar (quits in `--summon` mode) |
| `↑` `↓` / `Tab`        | move the candidate highlight            |
| `Backspace`            | delete last char                       |
| `Ctrl+U`               | clear the input                        |
| `Ctrl+C`               | quit (any state)                       |

The bar draws an **input box**, a **candidate list** and one status line
(no window title). With an empty input the list shows your **recent history
only** - newest first, deduplicated, up to **10 rows**. Typing switches it to
the ranked list of up to **5 candidates**: matching history first (newest
first), then concrete registered shortcuts by fuzzy score, which only fill
the slots history leaves. `<alias> <partial>` lists that alias's matching
shortcuts - a shortcut is a key such as `baidu` mapped to a concrete value,
and bare alias names never show up as rows of their own. `↑`/`↓`/`Tab` move
the highlight - the window scrolls once you reach its bottom - and `Enter`
runs the highlighted row: a history row replays its recorded pair, a shortcut
row runs the registered value. The whole alias table lives on the `/settings`
page.

## Built-in aliases

| Alias | Linux                                        | macOS             | Registered shortcuts |
|-------|----------------------------------------------|-------------------|----------------------|
| `br`  | `xdg-open {input} >/dev/null 2>&1 &`         | `open {input} >/dev/null 2>&1 &` | `baidu` -> `https://www.baidu.com`, `gm` -> `https://mail.google.com` |
| `cd`  | `@native clipboard` (built-in Rust backend)  | `@native clipboard` | -             |

Built-ins are exactly `br` and `cd` and they ship with their concrete content
registered - the command templates above *and* the shortcuts - so `br baidu`
works on a fresh machine with no store copy. Add more with `:arg` or the
settings page.

Commands run in their own process group, so the apps they launch survive the
bar dismissing itself (a summon success closes the terminal right away).
Long-lived apps should still be backgrounded *and* redirected
(`cmd <args> >/dev/null 2>&1 &`) so the bar returns and auto-dismisses — it
waits for the command's output pipes to close, so `&` alone leaves a job
holding them open (and costs you the stderr tail in failure messages).

## Custom aliases (`:` commands)

Type a line starting with `:` and press Enter:

```
:add <name>[,<trigger>...] <linux-cmd> [// <macos-cmd>]
:del <name>
:arg <name> <key> <value...>       # register a shortcut (key -> value)
:unarg <name> <key>                # remove a shortcut
:help
```

Examples:

```
:add gh,git-open xdg-open https://github.com/{input} // open https://github.com/{input}
:add clip-mac - // pbcopy @stdin      ('-' = not configured on linux)
:del gh
```

The built-in `cd` alias copies through the [`arboard`](https://crates.io/crates/arboard)
crate directly — no `xclip` / `wl-copy` / `xsel` / `pbcopy` binaries required
(Linux needs X11 or XWayland). Override it with `:add cd <cmd> // <cmd>` if
you prefer your own tool.

Placeholders:

* `{input}` — everything you typed after the alias, shell-quoted into the command.
* `@stdin` — your input is piped to the command's **stdin** instead.

Both platforms are first class: omitting `// <macos-cmd>` mirrors the linux
command (same rule the settings wizard uses), and at run time a command that
is missing or blank on the current platform falls back to the other
platform's command. Only when *both* are missing does the run fail with
`no command configured for <platform>`.

Runs go through `sh -c`, and only a run that actually worked is remembered: a
non-zero exit, a signal, a spawn failure or a missing command reports `✗ ...`
and leaves the history untouched. A template that backgrounds its own work
(`thunar {input} >/dev/null 2>&1 &`) would otherwise exit 0 the instant the job
is forked, so the bar appends a short probe: the launch stays detached, but a
job that is already gone after ~0.2 s (missing binary, bad argument) is reaped
for its real status and reported as a failure instead of a fake success.

Built-ins cannot be deleted; re-define them with `:add` to override.

### Shortcuts and triggers

A **shortcut** is a key mapped to one concrete value, and a **trigger** is an
extra word the alias answers to.

Give an alias shortcuts for the inputs you use all the time:

```
:arg br baidu https://www.baidu.com
```

Now `br baidu` opens `https://www.baidu.com`, while plain `br` and
`br <anything else>` behave exactly as before: typing `<alias> <key>` ranks
that key's value first, so `Enter` resolves it. The bar itself stays a
single input line - the registered shortcuts are listed on the `/settings`
page (`↳ baidu · https://www.baidu.com`). Values may contain spaces; history
keeps the raw text (`baidu`), so the shorthand stays replayable. Setting a
shortcut on a built-in alias materializes it as an overriding user
definition.

Triggers live on the alias itself, comma-separated after its name:

```
:add gh,git-open xdg-open https://github.com/{input}
```

Here `gh` is the name and `git-open` is a trigger, so both `gh me` and
`git-open me` run the same command. Triggers are alternate spellings, never
rows: the candidate list only ever shows history and concrete shortcuts.

Both kinds are managed on the `/settings` page (or from the colon commands
above).

## Command palette

Press **alt+d** while the bar is shown to drop down a selectable list of the
built-in `:`/`/` commands:

* `↑`/`↓` (or `Tab`) move the selection.
* `Enter` accepts. Complete commands run immediately (`:help` shows the
  colon-command help, `/settings` opens the settings page); commands that
  still need arguments (`:add`, `:del`, `:arg`, `:unarg`) prefill the input
  with a trailing space - type the rest and press Enter.
* `Esc` closes the palette; so does typing (the character is inserted
  normally).

The key is configurable and defaults to the wake key. While the bar is
**hidden** it still wakes the bar, and in `--summon` mode it still quits -
the wake key keeps priority there - so give the palette a distinct key when
you want it to act on its own:

```sh
xconsoler --set-command-key ctrl+k   # persist (validated, canonical)
xconsoler --command-key ctrl+k       # this run only, not persisted
```

## Settings page (`/settings`)

Type `/settings` and press Enter for a full-screen alias manager
(`Esc` or `q` returns to the bar):

The table has one row per alias and five columns —
`name | triggers | linux | macos | shortcuts` (missing commands show `—`,
long commands are truncated with `…`):

* `↑`/`↓` (or `j`/`k`) move the selection, `Enter`/`→` expands an alias to
  show its triggers and shortcuts (triggers first, then shortcuts), `←`
  collapses.
* `n` — wizard for a new alias: name → triggers (comma-separated, may be
  empty) → linux command (use `{input}` where the input goes) → macos
  command (empty = same as linux).
* `s` — wizard for a new shortcut on the selected alias (or on the row of
  one of its triggers/shortcuts, which routes to that alias): key → value.
* `t` — wizard for a new trigger on the selected alias.
* `e` — edit the selected alias's linux and macos commands; both steps are
  prefilled with the current values, `Ctrl+U` clears a field and a blank
  macos keeps mirroring linux.
* `d` — delete the selection: a shortcut row drops just that shortcut, a
  trigger row just that trigger, an alias row the whole alias (built-ins
  refuse; override them instead).

Every change is saved to `store.json` immediately.

## Storage

* Store lives at `<config_dir>/xconsoler/store.json` (override with `--store`).
* Config carries `wake_key` and `command_key` (both default `alt+d`) in the
  same file; stores written before a key existed load with its default.
  Change them with `xconsoler --set-wake-key <spec>` and
  `xconsoler --set-command-key <spec>`.
* Inputs are stored as **base64 of the plain text**, so quotes/unicode/newlines
  round-trip safely and nothing secret-looking is kept in cleartext.
* History: up to **10 000** entries, newest first, deduplicated per
  `alias + input` (re-running moves the entry to the top). **Failed runs are
  never recorded** — see the placeholder section above.
* Matching: fuzzy over `label + input` for history, and over
  `name + triggers + key + value` for shortcuts; an empty input lists the
  recent history only (up to 10 rows), while a typed query ranks up to 5
  candidates - matching history first (newest first), then concrete
  shortcuts by fuzzy score filling only the slots history leaves.

## Global wake-up

True global binding needs a multiplexer. With tmux (`.tmux.conf`):

```
bind -n M-d run -b 'xconsoler'
```

`run -b` keeps tmux responsive while the launcher runs in a background pane;
press `Esc` to hide it, `Ctrl+C` to quit. Without tmux, use the
SSH summon flow below, or just run `xconsoler` in a spare terminal/tab and
use `alt+d` to wake the bar and `Esc` to hide it again.

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
cursor `#c8c093` for the block cursor, `#2d4f67` for the selected row on the
`/settings` page (the bar paints nothing but that cursor block).
Changing the theme means changing those consts in `scripts/gen-logo.py` and
re-running it. `xc-icon` installs each size twice — as `xconsoler` (menu entry)
and as `XConsoler` (the WM_CLASS `xc-bar` gives the summoned window, which is how
the taskbar finds the icon). Regenerating needs Pillow (`pip install pillow`).

`xc-bar` spawns a `gnome-terminal` window (`--class=XConsoler`; `xterm` when
gnome-terminal is missing) with a dedicated GNOME Terminal profile over the
pinned kanagawa palette (`background-color=#1f1f28`, `foreground-color=#dcdcdc`,
`use-theme-colors=false`) because the launcher paints no background of its
own: the bar draws the 3-row input box, the candidate list and one status
row, so every cell it does not paint is the host terminal's translucent
background. The window is a **quarter of the screen wide**, centred, with its
top edge following the mouse cursor - 52x16 cells on a 1920px screen.
It keeps **16 rows** so the default list fits: 3 for the box, 2 for the
list's own title/borders, the 10 recent-history rows, 1 for the status.
Fewer rows simply clamp the list (`XC_ROWS=14` shows 8), and `/settings`
still draws its table in place from ~8 rows up - below that it runs out of
room. `XC_ROWS=4` shrinks the summon to a slim strip, `XC_COLS=140` pins the
old wide bar (and skips the pixel resize) when `/settings` wants the room. The requested **0.7 alpha** is pinned as
`background-transparency-percent=30`: the host terminal blends 70% bar with
30% desktop showing through. Override it with `XC_ALPHA=0.7` (0..1 opacity)
or `XC_TRANS=45` (VTE transparency percent, wins over `XC_ALPHA`).

**No window header**: the profile also turns gnome-terminal's client-side
headerbar off (a global `org.gnome.Terminal.Legacy.Settings` key, written
only when it is not already `false`), and a detached `xdotool` poll then
strips the WM frame that GTK falls back to (`_MOTIF_WM_HINTS`
decorations=0) and re-anchors the bar at the requested geometry. Profile
setup is idempotent and falls back to a default-coloured window (one-line
stderr warning) when gsettings/dconf are unavailable. It then runs
`xconsoler --summon` inside that window (the `xterm` fallback gets the same
geometry). Calling it again while the bar is up kills the bar instead of
opening a second one, and a successful run dismisses it automatically,
Spotlight-style.

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
