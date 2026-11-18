//! Small bounded per-process cache. Every hit revalidates DB consent revision.
//! Cache is an optimization, never an authorization source.
use super::Fact;
use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{Duration, Instant},
};
#[derive(Clone)]
struct Entry {
    facts: Vec<Fact>,
    rev: i64,
    expiration: Instant,
}
struct Inner {
    map: HashMap<String, Entry>,
    order: VecDeque<String>,
}
pub struct MemoryCache {
    inner: Mutex<Inner>,
    max: usize,
    ttl: Duration,
}
impl MemoryCache {
    pub fn new(max: usize, ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                order: VecDeque::new(),
            }),
            max: max.clamp(1, 128),
            ttl: ttl.min(Duration::from_secs(30)),
        }
    }
    pub fn get(&self, scope: &str, rev: i64) -> Option<Vec<Fact>> {
        let g = self.inner.lock().ok()?;
        let e = g.map.get(scope)?;
        (e.rev == rev && e.expiration > Instant::now()).then(|| e.facts.clone())
    }
    pub fn put(&self, scope: String, rev: i64, facts: Vec<Fact>) {
        if let Ok(mut g) = self.inner.lock() {
            g.order.retain(|s| s != &scope);
            while g.map.len() >= self.max && !g.map.contains_key(&scope) {
                if let Some(key) = g.order.pop_front() {
                    g.map.remove(&key);
                } else {
                    break;
                }
            }
            g.order.push_back(scope.clone());
            g.map.insert(
                scope,
                Entry {
                    rev,
                    facts,
                    expiration: Instant::now() + self.ttl,
                },
            );
        }
    }
    pub fn invalidate(&self, scope: &str) {
        if let Ok(mut g) = self.inner.lock() {
            g.map.remove(scope);
            g.order.retain(|s| s != scope);
        }
    }
    pub fn clear(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.map.clear();
            g.order.clear();
        }
    }
    pub fn len(&self) -> usize {
        self.inner.lock().map_or(0, |g| g.map.len())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn revision_fence() {
        let c = MemoryCache::new(1, Duration::from_secs(2));
        c.put("a".into(), 2, vec![]);
        assert!(c.get("a", 1).is_none());
        assert!(c.get("a", 2).is_some());
        c.invalidate("a");
        assert!(c.get("a", 2).is_none());
    }
}
