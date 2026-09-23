use std::collections::{HashSet, VecDeque};
use std::hash::Hash;

/// A set that remembers only the most recent `capacity` values.
#[derive(Debug)]
pub(crate) struct RecentSet<T> {
    order: VecDeque<T>,
    members: HashSet<T>,
    capacity: usize,
}

impl<T: Copy + Eq + Hash> RecentSet<T> {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            order: VecDeque::with_capacity(capacity),
            members: HashSet::with_capacity(capacity),
            capacity,
        }
    }

    /// Adds `value`. Returns `false` if it was already present.
    pub(crate) fn insert(&mut self, value: T) -> bool {
        if !self.members.insert(value) {
            return false;
        }
        self.order.push_back(value);
        if self.order.len() > self.capacity
            && let Some(oldest) = self.order.pop_front()
        {
            self.members.remove(&oldest);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_repeats() {
        let mut set = RecentSet::new(3);
        assert!(set.insert(1));
        assert!(!set.insert(1));
    }

    #[test]
    fn forgets_oldest_beyond_capacity() {
        let mut set = RecentSet::new(2);
        set.insert(1);
        set.insert(2);
        set.insert(3); // evicts 1
        assert!(set.insert(1));
        assert!(!set.insert(3));
    }
}
