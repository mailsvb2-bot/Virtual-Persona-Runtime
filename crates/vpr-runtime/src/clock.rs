use std::fmt::Debug;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) trait RuntimeClock: Debug + Send + Sync {
    fn now_millis(&self) -> Option<u64>;
}

#[derive(Debug, Default)]
pub(crate) struct SystemClock;

impl RuntimeClock for SystemClock {
    fn now_millis(&self) -> Option<u64> {
        let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
        u64::try_from(elapsed.as_millis()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::{RuntimeClock, SystemClock};

    #[test]
    fn system_clock_returns_unix_millis() {
        assert!(SystemClock.now_millis().is_some());
    }
}
