//! Non-PII counters; never log memory content, credentials or user IDs.
use std::sync::atomic::{AtomicU64,Ordering};
#[derive(Default)] pub struct MemoryTelemetry {
    pub read_ok:AtomicU64,pub read_error:AtomicU64,pub write_ok:AtomicU64,
    pub write_error:AtomicU64,pub cache_hit:AtomicU64,pub revoked:AtomicU64,
    pub outbox_deferred:AtomicU64,pub recall_micros:AtomicU64,
}
#[derive(Clone,Copy,Debug)]pub struct MemoryMetricSnapshot {
    pub read_ok:u64,pub read_error:u64,pub write_ok:u64,pub write_error:u64,
    pub cache_hit:u64,pub revoked:u64,pub outbox_deferred:u64,pub recall_micros:u64,
}
impl MemoryTelemetry {
    pub fn snapshot(&self)->MemoryMetricSnapshot {
        let l=|a:&AtomicU64|a.load(Ordering::Relaxed);
        MemoryMetricSnapshot{read_ok:l(&self.read_ok),read_error:l(&self.read_error),
            write_ok:l(&self.write_ok),write_error:l(&self.write_error),cache_hit:l(&self.cache_hit),
            revoked:l(&self.revoked),outbox_deferred:l(&self.outbox_deferred),recall_micros:l(&self.recall_micros)}
    }
    pub fn record_read(&self,ok:bool,elapsed:std::time::Duration){
       let target=if ok {&self.read_ok}else{&self.read_error}; target.fetch_add(1,Ordering::Relaxed);
       self.recall_micros.fetch_add(elapsed.as_micros().min(u64::MAX as u128) as u64,Ordering::Relaxed);
    }
}
