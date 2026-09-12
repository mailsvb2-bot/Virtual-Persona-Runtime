use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

use vpr_integration::CancellationProbe;

use crate::clock::RuntimeClock;

#[derive(Debug, Clone, Default)]
pub(crate) struct TurnCancellation {
    cancelled: Arc<AtomicBool>,
}

impl TurnCancellation {
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    #[must_use]
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn downgrade(&self) -> Weak<AtomicBool> {
        Arc::downgrade(&self.cancelled)
    }
}

impl CancellationProbe for TurnCancellation {
    fn is_cancelled(&self) -> bool {
        self.is_cancelled()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderCancellation {
    turn: TurnCancellation,
    clock: Arc<dyn RuntimeClock>,
    expires_at_millis: Option<u64>,
}

impl ProviderCancellation {
    #[must_use]
    pub(crate) fn new(
        turn: TurnCancellation,
        clock: Arc<dyn RuntimeClock>,
        expires_at_millis: Option<u64>,
    ) -> Self {
        Self {
            turn,
            clock,
            expires_at_millis,
        }
    }
}

impl CancellationProbe for ProviderCancellation {
    fn is_cancelled(&self) -> bool {
        if self.turn.is_cancelled() {
            return true;
        }
        let Some(expires_at_millis) = self.expires_at_millis else {
            return false;
        };
        let Some(now_millis) = self.clock.now_millis() else {
            return true;
        };
        now_millis >= expires_at_millis
    }
}
