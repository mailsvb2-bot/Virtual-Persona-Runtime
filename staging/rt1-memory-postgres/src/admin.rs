//! Strictly aggregated administrative view: no user identifiers, memory facts, secrets or text.
use super::{TextMemory,MemoryError,db};
#[derive(Debug,Clone)]pub struct MemoryAdminSnapshot {
 pub active_subjects:i64,pub facts:i64,pub queued_writes:i64,
 pub expired_facts:i64,pub unreconciled_erasure_requests:i64,
}
impl TextMemory {
 pub fn admin_snapshot(&self)->Result<MemoryAdminSnapshot,MemoryError>{
    let mut c=self.conn()?;
    let row=c.query_one("SELECT
       (SELECT count(*) FROM vpr_memory.visitor_consent WHERE enabled=true),
       (SELECT count(*) FROM vpr_memory.facts),
       (SELECT count(*) FROM vpr_memory.write_outbox WHERE done_at IS NULL),
       (SELECT count(*) FROM vpr_memory.facts WHERE expires_at<=now()),
       (SELECT count(*) FROM vpr_memory.erasure_tombstones WHERE reconciled_at IS NULL)", &[]).map_err(db)?;
    Ok(MemoryAdminSnapshot {active_subjects:row.get(0),facts:row.get(1),queued_writes:row.get(2),expired_facts:row.get(3),unreconciled_erasure_requests:row.get(4)})
 }
 pub fn flush_metric(&self,metric:&str,count:u64)->Result<(),MemoryError>{
    if metric.is_empty()||metric.len()>64||!metric.bytes().all(|b|b.is_ascii_lowercase()||b==b'_'){return Err(MemoryError::InvalidInput)}
    let mut c=self.conn()?;
    c.execute("INSERT INTO vpr_memory.memory_metrics(metric,count) VALUES($1,$2)
      ON CONFLICT(metric_day,metric) DO UPDATE SET count=vpr_memory.memory_metrics.count+EXCLUDED.count",&[&metric,&(count.min(i64::MAX as u64) as i64)]).map_err(db)?;Ok(())
 }
}
