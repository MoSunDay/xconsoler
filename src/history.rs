//! History recording on top of [`crate::storage::Store`].

use crate::storage::{HistoryEntry, Store, MAX_HISTORY};

/// Record an execution into `store.history`.
///
/// Dedup is by the entry key (`alias` + NUL + plain input): an existing
/// entry with the same key is removed first. The fresh entry is inserted at
/// index 0 (newest first) and the list is truncated to [`MAX_HISTORY`].
/// Note: [`HistoryEntry::new`] always produces the canonical base64 form, so
/// comparing `input_b64` is equivalent to comparing the decoded input while
/// avoiding an O(n) decode per element.
pub fn record(store: &mut Store, alias: &str, input: &str, ts: u64) {
    let entry = HistoryEntry::new(alias, input, ts);
    store
        .history
        .retain(|e| e.alias != alias || e.input_b64 != entry.input_b64);
    store.history.insert(0, entry);
    store.history.truncate(MAX_HISTORY);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_entry_lands_at_head() {
        let mut store = Store::default();
        record(&mut store, "br", "a", 1);
        record(&mut store, "br", "b", 2);
        assert_eq!(store.history.len(), 2);
        assert_eq!(store.history[0].input(), "b");
        assert_eq!(store.history[1].input(), "a");
    }

    #[test]
    fn duplicate_moves_to_head_with_new_ts() {
        let mut store = Store::default();
        record(&mut store, "br", "a", 1);
        record(&mut store, "br", "b", 2);
        record(&mut store, "br", "a", 99);
        assert_eq!(store.history.len(), 2);
        assert_eq!(store.history[0].input(), "a");
        assert_eq!(store.history[0].ts, 99);
        assert_eq!(store.history[1].input(), "b");
    }

    #[test]
    fn same_input_different_alias_not_deduped() {
        let mut store = Store::default();
        record(&mut store, "br", "a", 1);
        record(&mut store, "cd", "a", 2);
        assert_eq!(store.history.len(), 2);
    }

    #[test]
    fn capacity_truncates_oldest() {
        let mut store = Store::default();
        for i in 0..=MAX_HISTORY {
            record(&mut store, "br", &format!("i{i}"), i as u64);
        }
        assert_eq!(store.history.len(), MAX_HISTORY);
        assert_eq!(store.history[0].input(), format!("i{MAX_HISTORY}"));
        // oldest entry ("i0") dropped, "i1" is now the tail
        assert_eq!(store.history.last().unwrap().input(), "i1");
        assert!(store.history.iter().all(|e| e.input() != "i0"));
    }
}
