use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

use vpr_integration::CancellationProbe;

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
