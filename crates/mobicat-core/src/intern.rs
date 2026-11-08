use std::collections::HashMap;
use std::sync::Arc;

/// Deduplicates repeated strings (tags, subsystems) so entries share one allocation.
///
/// Bounded: when it grows past `limit` distinct strings it starts over, so a
/// source that emits unbounded distinct tags cannot grow memory without limit.
pub struct Interner {
    map: HashMap<Box<str>, Arc<str>>,
    limit: usize,
}

impl Default for Interner {
    fn default() -> Self {
        Self::with_limit(8192)
    }
}

impl Interner {
    pub fn with_limit(limit: usize) -> Self {
        Self {
            map: HashMap::new(),
            limit,
        }
    }

    pub fn get(&mut self, s: &str) -> Arc<str> {
        if let Some(a) = self.map.get(s) {
            return a.clone();
        }
        if self.map.len() >= self.limit {
            self.map.clear();
        }
        let a: Arc<str> = Arc::from(s);
        self.map.insert(s.into(), a.clone());
        a
    }

    /// Like `get`, but empty strings become `None`.
    pub fn opt(&mut self, s: &str) -> Option<Arc<str>> {
        (!s.is_empty()).then(|| self.get(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_allocations_and_stays_bounded() {
        let mut i = Interner::with_limit(2);
        let a = i.get("a");
        let a2 = i.get("a");
        assert!(Arc::ptr_eq(&a, &a2));
        i.get("b");
        i.get("c"); // over the limit: starts over
        assert!(i.map.len() <= 2);
        assert_eq!(i.opt(""), None);
    }
}
