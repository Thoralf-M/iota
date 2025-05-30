// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::{cmp::Ordering, collections::VecDeque, hash::Hash, sync::Arc};

use iota_types::base_types::SequenceNumber;
use moka::sync::Cache as MokaCache;
use parking_lot::Mutex;

/// CachedVersionMap is a map from version to value, with the additional
/// constraints:
/// - The key (SequenceNumber) must be monotonically increasing for each insert.
///   If a key is inserted that is less than the previous key, it results in an
///   assertion failure.
/// - Similarly, only the item with the least key can be removed.
/// - The intent of these constraints is to ensure that there are never gaps in
///   the collection, so that membership in the map can be tested by comparing
///   to both the highest and lowest (first and last) entries.
#[derive(Debug)]
pub struct CachedVersionMap<V> {
    values: VecDeque<(SequenceNumber, V)>,
}

impl<V> Default for CachedVersionMap<V> {
    fn default() -> Self {
        Self {
            values: VecDeque::new(),
        }
    }
}

impl<V> CachedVersionMap<V> {
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn insert(&mut self, version: SequenceNumber, value: V) {
        if !self.values.is_empty() {
            let back = self.values.back().unwrap().0;
            assert!(
                back < version,
                "version must be monotonically increasing ({} < {})",
                back,
                version
            );
        }
        self.values.push_back((version, value));
    }

    pub fn all_versions_lt_or_eq_descending<'a>(
        &'a self,
        version: &'a SequenceNumber,
    ) -> impl Iterator<Item = &'a (SequenceNumber, V)> {
        self.values.iter().rev().filter(move |(v, _)| v <= version)
    }

    pub fn get(&self, version: &SequenceNumber) -> Option<&V> {
        for (v, value) in self.values.iter().rev() {
            match v.cmp(version) {
                Ordering::Less => return None,
                Ordering::Equal => return Some(value),
                Ordering::Greater => (),
            }
        }

        None
    }

    /// returns the newest (highest) version in the map
    pub fn get_highest(&self) -> Option<&(SequenceNumber, V)> {
        self.values.back()
    }

    /// returns the oldest (lowest) version in the map
    pub fn get_least(&self) -> Option<&(SequenceNumber, V)> {
        self.values.front()
    }

    // pop items from the front of the collection until the size is <= limit
    pub fn truncate_to(&mut self, limit: usize) {
        while self.values.len() > limit {
            self.values.pop_front();
        }
    }

    // remove the value if it is the first element in values.
    pub fn pop_oldest(&mut self, version: &SequenceNumber) -> Option<V> {
        let oldest = self.values.pop_front()?;
        // if this assert fails it indicates we are committing transaction data out
        // of causal order
        assert_eq!(oldest.0, *version, "version must be the oldest in the map");
        Some(oldest.1)
    }
}

// an iterator adapter that asserts that the wrapped iterator yields elements in
// order
pub(super) struct AssertOrdered<I: Iterator> {
    iter: I,
    last: Option<I::Item>,
}

impl<I: Iterator> AssertOrdered<I> {
    fn new(iter: I) -> Self {
        Self { iter, last: None }
    }
}

impl<I: IntoIterator> From<I> for AssertOrdered<I::IntoIter> {
    fn from(iter: I) -> Self {
        Self::new(iter.into_iter())
    }
}

impl<I: Iterator> Iterator for AssertOrdered<I>
where
    I::Item: Ord + Copy,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.iter.next();
        if let Some(next) = next {
            if let Some(last) = &self.last {
                assert!(*last < next, "iterator must yield elements in order");
            }
            self.last = Some(next);
        }
        next
    }
}

// Could just use the Ord trait but I think it would be confusing to overload it
// in that way.
pub trait IsNewer {
    fn is_newer_than(&self, other: &Self) -> bool;
}

pub struct MonotonicCache<K, V> {
    cache: MokaCache<K, Arc<Mutex<V>>>,
}

impl<K, V> MonotonicCache<K, V>
where
    K: Hash + Eq + Send + Sync + Copy + 'static,
    V: IsNewer + Clone + Send + Sync + 'static,
{
    pub fn new(cache_size: u64) -> Self {
        Self {
            cache: MokaCache::builder().max_capacity(cache_size).build(),
        }
    }

    pub fn get(&self, key: &K) -> Option<Arc<Mutex<V>>> {
        self.cache.get(key)
    }

    // Update the cache with guaranteed monotonicity. That is, if there are N
    // calls to the this function from N threads, the write with the newest value
    // will win the race regardless of what ordering the writes occur in.
    //
    // Caller should log the insert with trace! and increment the appropriate
    // metric.
    pub fn insert(&self, key: &K, value: V) {
        // Warning: tricky code!
        let entry = self
            .cache
            .entry(*key)
            // only one racing insert will call the closure
            .or_insert_with(|| Arc::new(Mutex::new(value.clone())));

        // We may be racing with another thread that observed an older version of value
        if !entry.is_fresh() {
            // !is_fresh means we lost the race, and entry holds the value that was
            // inserted by the other thread. We need to check if we have a more recent value
            // than the other reader.
            let mut entry = entry.value().lock();
            if value.is_newer_than(&entry) {
                *entry = value;
            }
        }
    }

    pub fn invalidate(&self, key: &K) {
        self.cache.invalidate(key);
    }

    #[cfg(test)]
    pub fn contains_key(&self, key: &K) -> bool {
        self.cache.contains_key(key)
    }

    pub fn invalidate_all(&self) {
        self.cache.invalidate_all();
    }

    pub fn is_empty(&self) -> bool {
        self.cache.iter().next().is_none()
    }
}

#[cfg(test)]
mod tests {
    use iota_types::base_types::SequenceNumber;

    use super::*;

    // Helper function to create a SequenceNumber for simplicity
    fn seq(num: u64) -> SequenceNumber {
        SequenceNumber::from(num)
    }

    #[test]
    fn insert_and_get_last() {
        let mut map = CachedVersionMap::default();
        let version1 = seq(1);
        let version2 = seq(2);
        map.insert(version1, "First");
        map.insert(version2, "Second");

        let last = map.get_highest().unwrap();
        assert_eq!(last, &(version2, "Second"));
    }

    #[test]
    #[should_panic(expected = "version must be monotonically increasing")]
    fn insert_with_non_monotonic_version() {
        let mut map = CachedVersionMap::default();
        let version1 = seq(2);
        let version2 = seq(1);
        map.insert(version1, "First");
        map.insert(version2, "Second"); // This should panic
    }

    #[test]
    fn remove_first_item() {
        let mut map = CachedVersionMap::default();
        let version1 = seq(1);
        let version2 = seq(2);
        map.insert(version1, "First");
        map.insert(version2, "Second");

        let removed = map.pop_oldest(&version1);
        assert_eq!(removed, Some("First"));
        assert!(!map.values.iter().any(|(v, _)| *v == version1));
    }

    #[test]
    #[should_panic(expected = "version must be the oldest in the map")]
    fn remove_second_item_panics() {
        let mut map = CachedVersionMap::default();
        let version1 = seq(1);
        let version2 = seq(2);
        map.insert(version1, "First");
        map.insert(version2, "Second");

        let removed = map.pop_oldest(&version2);
        assert_eq!(removed, Some("Second"));
        assert!(!map.values.iter().any(|(v, _)| *v == version2));
    }

    #[test]
    fn insert_into_empty_map() {
        let mut map = CachedVersionMap::default();
        map.insert(seq(1), "First");
        assert_eq!(map.values.len(), 1);
    }

    #[test]
    fn remove_from_empty_map_returns_none() {
        let mut map: CachedVersionMap<&str> = CachedVersionMap::default();
        assert_eq!(map.pop_oldest(&seq(1)), None);
    }

    #[test]
    #[should_panic(expected = "version must be the oldest in the map")]
    fn remove_nonexistent_item() {
        let mut map = CachedVersionMap::default();
        map.insert(seq(1), "First");
        assert_eq!(map.pop_oldest(&seq(2)), None);
    }

    #[test]
    fn all_versions_lt_or_eq_descending_with_existing_version() {
        let mut map = CachedVersionMap::default();
        map.insert(seq(1), "First");
        map.insert(seq(2), "Second");
        let two = seq(2);
        let result: Vec<_> = map.all_versions_lt_or_eq_descending(&two).collect();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, seq(2));
        assert_eq!(result[1].0, seq(1));

        let one = seq(1);
        let result: Vec<_> = map.all_versions_lt_or_eq_descending(&one).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, seq(1));
    }

    #[test]
    fn get_existing_item() {
        let mut map = CachedVersionMap::default();
        map.insert(seq(1), "First");
        let item = map.get(&seq(1));
        assert_eq!(item, Some(&"First"));
    }

    #[test]
    fn get_item_not_in_map_returns_none() {
        let mut map = CachedVersionMap::default();
        map.insert(seq(1), "First");
        assert_eq!(map.get(&seq(2)), None);
    }

    #[test]
    fn truncate_map_to_smaller_size() {
        let mut map = CachedVersionMap::default();
        for i in 1..=5 {
            map.insert(seq(i), format!("Item {}", i));
        }
        map.truncate_to(3);
        assert_eq!(map.values.len(), 3);
        assert_eq!(map.values.front().unwrap().0, seq(3));
    }

    #[test]
    fn get_last_on_empty_map() {
        let map: CachedVersionMap<&str> = CachedVersionMap::default();
        assert!(map.get_highest().is_none());
    }

    #[test]
    fn test_assert_order() {
        let iter = AssertOrdered::from(1..=10);
        let result: Vec<_> = iter.collect();
        assert_eq!(result, (1..=10).collect::<Vec<_>>());
    }

    #[test]
    #[should_panic(expected = "iterator must yield elements in order")]
    fn test_assert_order_panics() {
        let iter = AssertOrdered::from(vec![1, 3, 2]);
        let _ = iter.collect::<Vec<_>>();
    }
}
