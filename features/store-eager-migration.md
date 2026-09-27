# Eager store migration on load (2026-09-27)

User report: "saved settings/cd content is STILL plaintext" after v5
(3dc9034) supposedly base64-encoded shortcut values.

## Diagnosis
- No write path ever stored plaintext post-v5: every save funnels
  through `storage::save` → `serialize_shortcuts` (src/storage.rs).
  Proven by sandboxing every deployed binary against a crafted store.
- The plaintext files were stale pre-v5 stores, never re-saved: v5
  encoded LAZILY (only on next save), and the TUI saves only on clean
  exit (src/main.rs:236). Local ~/xconsoler/store.json was v3, last
  written Sep 25 23:00 — before v5 existed.
- Settings-edit round-trip tests could not catch plaintext on disk:
  `decode_b64` falls back to the raw string, so reload assertions pass
  either way.

## Fix (78577fa)
- `load()` materializes the migration: when the DISK version is older
  than the current schema, the migrated store is written back
  best-effort (same pattern as the legacy-move write-back). A
  newer-than-current file is never rewritten.
- Regression tests assert RAW disk bytes (base64 present, plaintext
  absent) through `load()` and the settings wizard/edit paths
  (src/storage/version_tests.rs, src/settings_apply.rs). 472 tests.

## Fleet (2026-09-27)
- Local + node02 (192.168.31.57): binary refreshed, v3 stores migrated
  to v6 base64 (backups: /tmp/local-store-v3.bak, node02
  /tmp/node02-store-v3.bak). amos (192.168.31.196): rename-swap
  upgrade (scp hits ETXTBSY while the bar runs); its store was
  already v6.
- `baidu.com` visible in store.json is the `browser` alias's COMMAND
  template default (`${url:-...}`), not a shortcut value — commands
  are never encoded by design.
