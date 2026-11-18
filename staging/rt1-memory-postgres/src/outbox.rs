//! Bounded durable outbox. Durable enqueue is confirmed before claiming a note saved.
//! The worker is explicitly driven by the existing app's maintenance loop, not a second brain.
use super::{TextMemory,StablePrincipal,MemoryError,db,advisory_scope_lock,ensure_consent,MemorySource,valid_id,hex_sha256};
#[derive(Clone,Debug)]pub struct OutboxReceipt {pub operation_id:String,pub durable:bool}
impl TextMemory {
    /// Durable enqueue. After commit no raw audio or complete transcript is retained.
    pub fn enqueue_explicit_fact(&self,p:&StablePrincipal,op:&str,key:&str,content:&str,days:i32)
        ->Result<OutboxReceipt,MemoryError>{
        if !valid_id(op,96)||!valid_id(key,96)||content.trim().is_empty()||content.len()>2048||content.contains('\0')||!(1..=3650).contains(&days){return Err(MemoryError::InvalidInput)}
        let digest=hex_sha256(&format!("{}\0{}\0{}\0{}",key,content,"visitor_explicit",days));
        let mut c=self.conn()?;let mut tx=c.transaction().map_err(db)?;
        advisory_scope_lock(&mut tx,p)?;ensure_consent(&mut tx,p)?;
        let grant_row=tx.query_one(
            "SELECT grant_epoch, consent_revision FROM vpr_memory.visitor_consent WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 AND enabled=true",
            &p.args(),
        ).map_err(db)?;
        let grant_epoch:i64=grant_row.get(0);
        let consent_revision:i64=grant_row.get(1);
        let old=tx.query_opt("SELECT content_hash FROM vpr_memory.write_outbox WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 AND operation_id=$4",
         &[&p.workspace_id,&p.visitor_id,&p.persona_id,&op]).map_err(db)?;
        if let Some(row)=old {if row.get::<_,String>(0)!=digest{return Err(MemoryError::Conflict)}} else {
          // Replaying the same queued request is allowed even at capacity, but
          // a genuinely new request must respect the bounded backlog.
          let count:i64=tx.query_one("SELECT count(*) FROM vpr_memory.write_outbox WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 AND done_at IS NULL",&p.args()).map_err(db)?.get(0);
          if count>=32{return Err(MemoryError::Capacity)}
          tx.execute("INSERT INTO vpr_memory.write_outbox(workspace_id,visitor_id,persona_id,operation_id,fact_key,content,content_hash,retention_days,grant_epoch,consent_revision)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
           &[&p.workspace_id,&p.visitor_id,&p.persona_id,&op,&key,&content,&digest,&days,&grant_epoch,&consent_revision]).map_err(db)?;
        }
        tx.commit().map_err(db)?;Ok(OutboxReceipt{operation_id:op.into(),durable:true})
    }
    /// Process one authorized scope in small batches. Failed items stay queued with capped retries.
    /// Idempotent final `remember` independently checks the still-current credential and consent.
    pub fn drain_scope_outbox(&self,p:&StablePrincipal,max:i64)->Result<u64,MemoryError>{
        if !(1..=16).contains(&max){return Err(MemoryError::InvalidInput)}
        let mut c=self.conn()?;let mut tx=c.transaction().map_err(db)?;
        advisory_scope_lock(&mut tx,p)?;ensure_consent(&mut tx,p)?;
        let rows=tx.query("SELECT operation_id,fact_key,content,retention_days,grant_epoch FROM vpr_memory.write_outbox
          WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 AND done_at IS NULL AND next_attempt_at<=now()
          ORDER BY created_at LIMIT $4 FOR UPDATE SKIP LOCKED",
          &[&p.workspace_id,&p.visitor_id,&p.persona_id,&max]).map_err(db)?;
        // Do not hold pool connection while invoking nested remember(): release transaction first.
        let jobs:Vec<(String,String,String,i32,i64)>=rows.into_iter().map(|r|(r.get(0),r.get(1),r.get(2),r.get(3),r.get(4))).collect();
        tx.commit().map_err(db)?;drop(c);
        let mut done=0;
        for (id,key,value,days,grant_epoch) in jobs {
            // Authentication, grant epoch and the fact mutation are verified
            // under the SAME PostgreSQL scope advisory transaction lock.
            // Fact writes may advance consent_revision without invalidating
            // other queued notes from this grant.
            match self.remember_reviewed_in_grant(p,&id,&key,&value,days,Some(grant_epoch)){
                Ok(_)=>{self.discard_outbox(p,&id,grant_epoch)?;done+=1},
                Err(MemoryError::NotConsented)=>{
                    self.discard_outbox(p,&id,grant_epoch)?;
                    return Err(MemoryError::NotConsented)
                },
                Err(_)=>{self.defer_outbox(p,&id,grant_epoch)?;}
            }
        }
        Ok(done)
    }
    fn discard_outbox(&self,p:&StablePrincipal,op:&str,epoch:i64)->Result<(),MemoryError>{
        let mut c=self.conn()?;
        c.execute("DELETE FROM vpr_memory.write_outbox WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 AND operation_id=$4 AND grant_epoch=$5",
          &[&p.workspace_id,&p.visitor_id,&p.persona_id,&op,&epoch]).map_err(db)?;Ok(())
    }
    fn defer_outbox(&self,p:&StablePrincipal,op:&str,epoch:i64)->Result<(),MemoryError>{
        let mut c=self.conn()?;
        c.execute("UPDATE vpr_memory.write_outbox SET attempts=attempts+1,next_attempt_at=now()+LEAST(3600,power(2,LEAST(attempts,10))::int)*interval '1 second'
           WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 AND operation_id=$4 AND grant_epoch=$5",
          &[&p.workspace_id,&p.visitor_id,&p.persona_id,&op,&epoch]).map_err(db)?;Ok(())
    }
}
