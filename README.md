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
xconsoler --print-rows                     # print bar height in rows (used by scripts/xc-bar)
xconsoler --print-app 微信                  # print the app `app 微信` would launch
xconsoler --set-wake-key alt+j             # change + persist the wake key
xconsoler -h | --help                      # usage
```

Or install the tagged release straight from the repo, no checkout needed:

```sh
cargo install --git https://github.com/MoSunDay/xconsoler.git --tag v0.1.0 --locked
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
| `Ctrl+N` / `Ctrl+P`    | move the candidate highlight down/up    |
| `←` `→`                | move the text caret one char            |
| `Ctrl+B` / `Ctrl+F`    | move the text caret one char left/right |
| `Ctrl+←` `Ctrl+→` (or `Alt+B`/`Alt+F`) | move the caret one word left/right |
| `Home` / `End` (or `Ctrl+A`/`Ctrl+E`) | caret to line start/end  |
| `Backspace` (`Ctrl+H`) | delete the char before the caret        |
| `Delete`               | delete the char under the caret         |
| `Ctrl+W`               | delete the whitespace-delimited word before the caret |
| `Ctrl+U` / `Ctrl+K`    | kill from the caret to the line start/end |
| `Ctrl+T`               | transpose the two chars around the caret |
| `Ctrl+C` / `Ctrl+D`    | quit (any state)                        |

The input line has a real text caret, readline-style: tokens are inserted and
removed at the caret, not at the end, and the box scrolls sideways once the
text no longer fits so the caret stays visible.

The bar draws an **input box**, a **candidate list** and one status line
(no window title). With an empty input the list shows your **recent history
only** - newest first, deduplicated, up to **10 rows**. Typing switches it to
the ranked list of up to **5 candidates**: matching history first (newest
first), then concrete registered shortcuts by fuzzy score, which only fill
the slots history leaves. `<alias> <partial>` narrows those shortcut rows to
that alias's matching keys - a shortcut is a key such as `baidu` mapped to a
concrete value - while matching history still leads and bare alias names
never show up as rows of their own. `↑`/`↓`/`Tab` move the highlight - the
window scrolls once you reach its bottom - and `Enter` runs the highlighted
row: a history row replays its recorded pair, a shortcut row runs the
registered value. The whole alias table lives on the `/settings` page.

**The bar's frame is sized once and then stays put.** Its height carries the
stored history an empty bar lists (6 rows plus one per recent entry, 16 at
most) and is never less than room for the typed candidate set (11 rows for
the 5 candidates a query can rank). Typing a query, clearing it or replaying
a command only changes the candidate list's *contents*, so the window cannot
flicker under your fingers; the palette keeps one fixed height for its whole
open session and only `/settings` - a full page, not a bar - grows the
window. A plain `xconsoler` run shrinks the terminal it was started from just
like a summoned bar shrinks its own window; the size the window had at
startup (or the last one you picked by dragging it) comes back when the bar
exits. It first writes
the in-band request `ESC[8;<rows>;<cols>t`; terminals that ignore it (xterm
without `allowWindowOps`, VTE, alacritty) are covered under X11 by `xdotool
getactivewindow windowsize --usehints <cols> <rows>`, which sizes in character
cells and is skipped under Wayland, where the compositor decides. The fallback
only ever touches a window that provably belongs to this process tree: the
focused window's pid must be ours or one of our ancestors. A request is
repeated while the window does not report that height - a window still being
mapped or focused hears none of the first tries - a few times on consecutive
ticks and then every couple of seconds, up to eight asks per height, and the
bar stops as soon as the terminal reports it, while a resize from outside -
you dragging the window - wins and is adopted. Inside tmux the escape goes
through the pane's passthrough (`ESC P tmux; ...`) to the outer terminal,
which is asked for the pane height plus the status line; tmux 3.3+ needs
`allow-passthrough` on for that, which the bar switches on for its window and
puts back at exit, and a pane that shares its window with a split is left
alone. `XC_ROWS`/`XC_NO_FIT` override all of it as described under the desktop
hotkey below.

## Seeded aliases

| Alias | Linux                                        | macOS             | Registered shortcuts |
|-------|----------------------------------------------|-------------------|----------------------|
| `br`  | `xdg-open {input} >/dev/null 2>&1 &`         | `open {input} >/dev/null 2>&1 &` | `baidu` -> `https://www.baidu.com`, `gm` -> `https://mail.google.com` |
| `cd`  | `@native clipboard` (native Rust backend)    | `@native clipboard` | -             |
| `app` | `@native app` (installed applications)       | `@native app`       | -             |

Every fresh store seeds exactly `br`, `cd` and `app`, concrete content included
- the command templates above *and* the shortcuts - so `br baidu` works on a machine
with no store copy yet. They are ordinary store entries: edit or delete them
on the settings page (or with `:del`), and bring them back with `:add`. Add
more with `:arg` or the settings page.

Commands run in their own process group, so the apps they launch survive the
bar dismissing itself (a summon success closes the terminal right away). A
backgrounded template (`thunar {input} &`) gets its stdio detached from the bar
automatically, so a long-lived app cannot hold the bar's pipes open for its
whole lifetime; the price is that a backgrounded failure reports its exit code
without a stderr tail.

## Custom aliases (`:` commands)

Type a line starting with `:` and press Enter:

```
:add <name>[,<trigger>...] <linux-cmd> [// <macos-cmd>]
:del <name>
:arg <name> <key> <value...>       # register a shortcut (key -> value)
:unarg <name> <key>                # remove a shortcut
:import-chrome [<alias>]           # import Chrome bookmarks (default: br)
:help
```

Examples:

```
:add gh,git-open xdg-open https://github.com/{input} // open https://github.com/{input}
:add clip-mac - // pbcopy @stdin      ('-' = not configured on linux)
:del gh
```

The seeded `cd` alias copies through the [`arboard`](https://crates.io/crates/arboard)
crate directly — no `xclip` / `wl-copy` / `xsel` / `pbcopy` binaries required
(Linux needs X11 or XWayland). It reads its input as base64 and puts the
decoded text on the clipboard (plain text that is not valid base64 is copied
verbatim). History keeps the input as recorded — the base64 text — so the
history row shows the stored form and a replay decodes it again the same way.
Override it with `:add cd <cmd> // <cmd>` if you prefer your own tool.

The seeded `app` alias launches an installed application by name. It matches
the entries the desktop menu would show - the XDG data dirs, plus flatpak and
snap - by id, display name (and this locale's translation), the keywords the
entry carries, the executable's base name, and the pinyin initials of a
Chinese name, so `app xx` reaches 小小备忘录 (`xiaoxiao beiwanglu`) and `app wx`
reaches 微信. The name a single entry answers to launches it; when several
entries match, the bar lists them and launches nothing, so it never guesses
between two apps. On macOS the same alias hands the name to `open`, and an
input that looks like a URL passes through to the default browser.

Placeholders:

* `{input}` — everything you typed after the alias, shell-quoted into the command.
* `@stdin` — your input is piped to the command's **stdin** instead.

Both platforms are first class: omitting `// <macos-cmd>` mirrors the linux
command (the same both-platform behavior `:add` has always had — the
`/settings` wizard instead stores only the current platform's command), and
at run time a command that is missing or blank on the current platform falls
back to the other platform's command. Only when *both* are missing does the
run fail with `no command configured for <platform>`.

Runs go through `sh -c`, and only a run that actually worked is remembered: a
non-zero exit, a signal, a spawn failure or a missing command reports `✗ ...`
and leaves the history untouched. A template that backgrounds its own work
(`thunar {input} &`) would otherwise exit 0 the instant the job is forked, so
the bar appends a probe that waits for the job: a job that exits non-zero --
even a second later -- is reported as a failure, and a job still running after
roughly a second is reported as `↳ ... started (still running; not recorded)`
instead of a fake success. The launch stays detached either way, so a launcher
that takes its time (`xdg-open` before the browser is up) keeps running while
nothing enters the history until it is known to have worked.

Seeded aliases are ordinary store entries: edit or delete them, and `:add` brings a name back.

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
keeps the raw text (`baidu`), so the shorthand stays replayable. Shortcuts
are edited in place on the settings page (`e` on a shortcut row) or replaced
with `:arg`; aliases are plain store data, seeded ones included.

Triggers live on the alias itself, comma-separated after its name:

```
:add gh,git-open xdg-open https://github.com/{input}
```

Here `gh` is the name and `git-open` is a trigger, so both `gh me` and
`git-open me` run the same command. Triggers are alternate spellings, never
rows: the candidate list only ever shows history and concrete shortcuts.

Both kinds are managed on the `/settings` page (or from the colon commands
above).

### Chrome bookmarks

`:import-chrome` reads Chrome/Chromium's `Bookmarks` file and registers every
bookmark as a shortcut - `key` -> `url` - on the `br` alias, or on the alias
you name:

```
:import-chrome             # into br
:import-chrome docs        # ... into the docs alias instead
```

After it, `br <key>` opens the page exactly like a hand-written shortcut.
Re-running it is safe: keys and URLs that are already there are left alone,
only new bookmarks are appended, and an existing shortcut is never
overwritten. Keys are slugs of the bookmark title (`Rust std docs` ->
`rust-std-docs`; untitled or non-ASCII bookmarks fall back to the site's
host), and a key that is already taken becomes `<folder>-<key>` or gets a
`-2` suffix. Only `http(s)` bookmarks are imported, at most 300 per run.
The usual profiles are scanned (`google-chrome`, `chromium`, the snap and
flatpak trees; `Default` first) - point `XC_CHROME_BOOKMARKS` at one
`Bookmarks` file to override that. The bar reports how many rows landed (or
why the file could not be read). The target alias is the one in the store,
so `:import-chrome` with no argument fills the seeded `br` like any other.

## Command palette

Press **alt+d** while the bar is shown to drop down a selectable list of the
`:`/`/` commands. Typing `/` does the same for slash commands only: the list
opens as soon as the line starts with `/` and filters fuzzily as you type, so
`/sett` narrows straight to `/settings`:

* `↑`/`↓` (or `Tab`) move the selection.
* `Enter` accepts. Complete commands run immediately (`:help` shows the
  colon-command help, `/settings` opens the settings page); commands that
  still need arguments (`:add`, `:del`, `:arg`, `:unarg`) prefill the input
  with a trailing space - type the rest and press Enter.
* `Esc` closes the list; typing plain text also closes it (the character is
  inserted normally), while a `:`/`/` query keeps filtering it.

The key is configurable and defaults to the wake key. While the bar is
**hidden** it still wakes the bar, and in `--summon` mode it still quits -
the wake key keeps priority there - so give the palette a distinct key when
you want it to act on its own:

```sh
xconsoler --set-command-key ctrl+k   # persist (validated, canonical)
xconsoler --command-key ctrl+k       # this run only, not persisted
```

## Settings page (`/settings`)

Type `/settings` and press Enter - or just `/s`, picked from the fuzzy list -
for a full-screen alias manager (`Esc` or `q` returns to the bar;
`Ctrl+C`/`Ctrl+D` quits):

The table has one row per alias and four columns —
`name | triggers | <platform> | shortcuts`, where the command column is named
after the platform xconsoler runs on (`linux` on Linux, `macos` on a Mac).
Only that platform's stored command is shown: `—` means none is stored for
this platform (the run path still falls back to the other platform's
command), long commands are truncated with `…`, and the other platform's
field never appears on this page.

* `↑`/`↓` (or `j`/`k`) move the selection, `Enter`/`→` expands an alias to
  show its triggers and shortcuts (triggers first, then shortcuts), `←`
  collapses.
* `n` — wizard for a new alias: name → triggers (comma-separated, may be
  empty) → `<platform>` command (use `{input}` where the input goes).
  Exactly one command is collected and stored; at run time the other
  platform uses it as a fallback.
* `s` — wizard for a new shortcut on the selected alias (or on the row of
  one of its triggers/shortcuts, which routes to that alias): key → value.
* `t` — wizard for a new trigger on the selected alias.
* `e` — edit the selected row in place, prefilled with its current value:
  an alias's `<platform>` command, a shortcut's key and value, or a
  trigger's word. `Ctrl+U` kills from the caret back to the start of a field
  (`Ctrl+K` kills to the end). Editing an alias changes only the current
  platform's command; the other platform's stored command is left untouched
  (hand-edit `store.json` to change it).
* `d` — delete the selection: a shortcut row drops just that shortcut, a
  trigger row just that trigger, an alias row the whole alias, seeded ones
  included.

Every change is saved to `store.json` immediately.

## Storage

* Store lives at `~/xconsoler/store.json` (override with `--store`). A store
  left at the old `~/.config/xconsoler/store.json` path is picked up once,
  when the **default** path does not exist yet; a custom `--store` never
  falls back to it.
* The file carries `version` (currently `3`). Stores older than 2 hold only
  overrides of the seeded `br`/`cd`, so loading merges those defaults back in
  (same-name stored aliases replace them in place, other names are appended);
  legacy alias keys are normalized in memory. The new form reaches disk the
  next time the file is saved - any `/settings` edit, a recorded run,
  `--set-wake-key`/`--set-command-key`, or bar exit. A file from a **newer**
  build is loaded as-is and never overwritten.
* A corrupt `store.json` is moved to `store.json.corrupt` and replaced with a
  fresh seeded store immediately, so a damaged file never blocks startup; the
  bar reports the move on its status line. If aliases seem to have vanished,
  look for `store.json.corrupt` next to the store - nothing is deleted.
* Config carries `wake_key` and `command_key` (both default `alt+d`) in the
  same file; stores written before a key existed load with its default.
  Change them with `xconsoler --set-wake-key <spec>` and
  `xconsoler --set-command-key <spec>`.
* Aliases on disk speak the same vocabulary as the settings page: `triggers`
  holds the alternate trigger words and `shortcuts` maps a shortcut key to
  the input it expands to. Legacy keys still load: a `shortcuts` **array** is
  read as trigger words (unless `triggers` is present), an `args` map as the
  shortcut map (unless the `shortcuts` map is present), and `builtin` is
  ignored. A fresh entry looks like this (stores written by older builds are
  migrated on load, so nothing to do by hand):
  ```json
  {"name":"br","triggers":["b"],"linux":"xdg-open {input}","macos":null,
   "shortcuts":{"baidu":"https://www.baidu.com"}}
  ```
  Hand-editing is supported; keep the top-level `"version": 3`, because a file
  without it counts as pre-v2 and gets the seeded defaults merged back in on
  the next load. `/settings` only ever reads and writes the running
  platform's field, so the other platform's stored command stays exactly as
  hand-edited.
* Inputs are stored as **base64 of the plain text**, so quotes/unicode/newlines
  round-trip safely and nothing secret-looking is kept in cleartext. A `cd`
  run records the base64 text itself, so the history row shows the stored
  form while the clipboard gets the decoded text.
* History: up to **100** entries, newest first, deduplicated per
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
press `Esc` to hide it, `Ctrl+C`/`Ctrl+D` to quit. Without tmux, use the
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
top edge following the mouse cursor - 52 cells wide, 4..16 tall, on a 1920px
screen.

Its height is fixed for the session: room for the store's **recent history**
(the 3-row input box + the list's title/borders + the status/hints row + one
per recent entry, topping out at 16 for the 10 entries the list shows) and
never less than room for the typed candidate set, so a query cannot resize
the bar. A store that cannot be read (corrupt ones count as no history) or a
missing python3 means the old 16-row default. The TUI itself shrinks the list into whatever it is given: a framed
block while the border/title and a row fit, bare rows in a slim strip, with
the key hints the first thing to go - so `XC_ROWS=4` is a usable one-liner
strip. `/settings` still draws its table in place from ~8 rows up; below that
it runs out of room. `XC_ROWS` pins a fixed height, `XC_NO_FIT=1` opts out
of the window auto-fit entirely (the bar keeps the size the window was
opened with, even for `/settings`), `XC_COLS=140` pins the old
wide bar (and skips the pixel resize) when `/settings` wants the room, and
`XC_PRINT_ROWS=1` prints the computed height and exits. The requested **0.7
alpha** is pinned as `background-transparency-percent=30`: the host terminal
blends 70% bar with 30% desktop showing through. Override it with
`XC_ALPHA=0.7` (0..1 opacity) or `XC_TRANS=45` (VTE transparency percent,
wins over `XC_ALPHA`).

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

# or stage the whole thing on another machine over SSH (binary + helpers +
# a `--print-rows` smoke test there):
scripts/xc-deploy root@host            # --bin PATH / --no-desktop / [DEST]

# print the binding for your rc file (~/.bashrc):
xconsoler --print-bind                 # >> paste into ~/.bashrc
xconsoler --print-bind --shell zsh     # zsh flavour (bindkey + ZLE widget)
```

Over SSH the fit can only use the resize escape, so the *terminal in front of
you* decides: VTE/gnome-terminal, xterm (with `allowWindowOps`), kitty and
wezterm honour it, while several emulators ignore window ops by policy. Inside
tmux the request travels in the passthrough envelope, which needs tmux 3.3+
`allow-passthrough on` - the bar switches that on for its own window while it
runs, and restores the previous value on exit.

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
