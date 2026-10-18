# app scan performance — single pass, precomputed keys, disk cache (closed 2026-10-18)

Enter-time cost of `app <name>` was dominated by the cold-process full
desktop scan: every run re-read 100-400 `.desktop` files 4-5 times over
(`Hidden`/`launchable`/`TryExec`/`parse` passes), `score()` rebuilt each
entry's key list per query, and pinyin initials did 26 linear scans over
~70KB of CJK table per character. No layer cached anything (the bar process
dies after a successful run, so in-memory caching alone cannot amortize).

## What landed
- `desktop.rs` — `Entry.keys: Vec<String>` precomputed at parse; `parse`
  split into `collect` (single pass over `[Desktop Entry]` lines, now also
  reading `Hidden`/`TryExec`/`DBusActivatable`) + `entry_from` (label/names
  post-processing + keys). `score()` reads `entry.keys`; `keys()` stays a
  public pure function. `field`/`flag`/`launchable` removed (superseded).
- `desktop/scan.rs` — `scan_dirs` reads each file once and filters on the
  collected `Raw`; `scan()` Linux arm consults the disk cache.
- `desktop/cache.rs` (new) — `$HOME/xconsoler/app-scan.json` next to
  `store.json`: `VERSION` + per-dir fingerprint (dir path, dir mtime,
  per-`.desktop`-file name/mtime/len — file-level mtimes catch in-place
  edits that a dir-mtime-only stamp would miss) + serialized entries (keys
  and TryExec verdicts included). Hit = skip scan AND PATH probes; miss =
  full scan + atomic write-back (sibling `.tmp` + rename, storage.rs
  pattern). Any error (missing/corrupt/version/fingerprint) silently falls
  back to a full scan; empty first scan writes nothing.
- `pinyin.rs` — `OnceLock<HashMap<char, u32>>` memoized char→initial-bit
  table (pure builder over `GROUPS`); `acronyms()` dedups via `HashSet`.
- `.gitignore` — `/app-scan.json` (+ `.tmp`).

## Numbers (release, 300 synthetic + real dirs, zh_CN locale)
- old binary, every Enter: ~21 ms
- new, cold (cache miss): ~12 ms
- new, warm (cache hit): ~4 ms (≈5x)

## Gotchas
- `no_display` still means "NoDisplay or Hidden" on `Entry`; `collect`
  keeps them separate so the scan can drop `Hidden` while `NoDisplay`
  entries stay listed-but-flagged.
- Cache path comes from `storage::config_dir()`; on the dev box that IS the
  repo root, hence the gitignore entries.
- Entry literals in tests must fill `keys` (helpers do `e.keys = keys(&e)`),
  or `score()` sees only whole-name matches.
- Future-work notes (not done): `ranked_candidates` computed twice per
  frame (fit + draw), `wait_grace` sync-blocks up to 1s.

482 tests green, clippy + fmt clean.

## Target regression (root@192.168.31.196, 2026-10-18, HEAD `1145cc2`)
- 239 desktop files, md5 local/remote match (`1741037f…`)
- `--print-app` wall time incl. startup: baseline (no scan) ~3 ms,
  cold ~22 ms, warm ~6 ms → scan cost ~19 ms → ~3 ms
- root's cache built fine (245 entries, 6 dirs); user `m` builds theirs on
  first `app` Enter; corrupt-cache and add-file live checks passed on the
  dev box earlier the same day
