//! Śledzenie offsetów w locie i wyznaczanie najwyższego ciągłego offsetu gotowego do commitu.

use std::collections::{BTreeMap, HashMap};

/// Śledzi offsety w locie per partycja i wyznacza najwyższy ciągły offset gotowy do commitu.
/// Gwarantuje commitowanie ściśle w kolejności (bez przeskakiwania nieukończonych wiadomości — at-least-once).
#[derive(Debug, Default)]
pub struct OffsetTracker {
    partitions: HashMap<i32, BTreeMap<i64, bool>>,
}

impl OffsetTracker {
    pub fn track(&mut self, partition: i32, offset: i64) {
        self.partitions.entry(partition).or_default().insert(offset, false);
    }

    /// Oznacza offset jako przetworzony i zwraca najwyższy ciągły ukończony offset tej partycji (jeśli okno się przesunęło).
    pub fn mark_done(&mut self, partition: i32, offset: i64) -> Option<i64> {
        let map = self.partitions.entry(partition).or_default();
        if let Some(done) = map.get_mut(&offset) {
            *done = true;
        }
        let mut commit_offset = None;
        while let Some((&first_off, &is_done)) = map.iter().next() {
            if !is_done {
                break;
            }
            map.remove(&first_off);
            commit_offset = Some(first_off);
        }
        commit_offset
    }

    pub fn in_flight(&self) -> usize {
        self.partitions.values().map(BTreeMap::len).sum()
    }

    /// Zapomina partycję (rewokacja): wiadomości w locie przestają blokować/wpływać na commity.
    pub fn remove_partition(&mut self, partition: i32) {
        self.partitions.remove(&partition);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commits_only_contiguous_prefix() {
        let mut t = OffsetTracker::default();
        for o in 0..4 {
            t.track(0, o);
        }
        assert_eq!(t.mark_done(0, 2), None);
        assert_eq!(t.mark_done(0, 1), None);
        assert_eq!(t.mark_done(0, 0), Some(2));
        assert_eq!(t.in_flight(), 1);
        assert_eq!(t.mark_done(0, 3), Some(3));
        assert_eq!(t.in_flight(), 0);
    }

    #[test]
    fn remove_partition_drops_in_flight_only_for_that_partition() {
        let mut t = OffsetTracker::default();
        t.track(0, 5);
        t.track(0, 6);
        t.track(1, 7);
        t.remove_partition(0);
        assert_eq!(t.in_flight(), 1);
        assert_eq!(t.mark_done(1, 7), Some(7));
    }
}
