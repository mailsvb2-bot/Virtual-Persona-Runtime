use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

#[derive(Debug, Clone, Default)]
pub(crate) struct SessionExecutionGate {
    inner: Arc<RwLock<()>>,
}

impl SessionExecutionGate {
    pub(crate) fn read(&self) -> Result<RwLockReadGuard<'_, ()>, ()> {
        self.inner.read().map_err(|_| ())
    }

    pub(crate) fn write(&self) -> Result<RwLockWriteGuard<'_, ()>, ()> {
        self.inner.write().map_err(|_| ())
    }
}
