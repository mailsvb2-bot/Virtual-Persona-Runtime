//! Local runtime authorization fence. A lease spans provider egress; revoke blocks new ones.
//! Once information reaches a remote LLM it cannot be recalled retroactively.
use std::sync::{Mutex,Condvar};
use std::time::{Duration,Instant};
#[derive(Default)]struct State{revoked:bool,active:u32,revision:i64}
pub struct MemoryEgressFence{state:Mutex<State>,done:Condvar}
pub struct EgressLease<'a>{fence:&'a MemoryEgressFence,active:bool}
/// Owns the same permit across a real provider invocation without borrowing
/// its session's engine (the canonical turn may already hold an engine lock).
pub struct OwnedEgressLease{fence:std::sync::Arc<MemoryEgressFence>,active:bool}
#[derive(Debug,PartialEq,Eq)]pub enum StopResult {Drained,StillInFlight}
impl MemoryEgressFence {
 pub fn new()->Self{Self{state:Mutex::new(State::default()),done:Condvar::new()}}
 /// Call after a PostgreSQL consent_revision check immediately before outbound LLM.
 pub fn admit(&self,revision:i64)->Option<EgressLease<'_>>{
  let mut state=self.state.lock().ok()?;
  if state.revoked{return None}
  if state.revision!=0 && state.revision!=revision {
   if revision<state.revision || state.active!=0{return None}
   state.revision=revision;
  }
  if state.revision==0 {state.revision=revision;}
  state.active=state.active.checked_add(1)?;
  Some(EgressLease{fence:self,active:true})
 }
 /// Own an egress permit that can be returned through an object-safe port.
 pub fn admit_owned(self:&std::sync::Arc<Self>,revision:i64)->Option<OwnedEgressLease>{
  let mut state=self.state.lock().ok()?;
  if state.revoked{return None}
  if state.revision!=0 && state.revision!=revision {
   if revision<state.revision || state.active!=0{return None}
   state.revision=revision;
  }
  if state.revision==0{state.revision=revision;}
  state.active=state.active.checked_add(1)?;
  Some(OwnedEgressLease{fence:std::sync::Arc::clone(self),active:true})
 }
 /// Mark revoked first, then wait for active provider requests to finish/cancel.
 /// Must be paired with canonical RT1 interrupt / remote STOP owner.
 pub fn revoke(&self,timeout:Duration)->StopResult {
  let Ok(mut state)=self.state.lock() else{return StopResult::StillInFlight};
  state.revoked=true;
  let until=Instant::now()+timeout;
  while state.active>0 {
   let remaining=until.saturating_duration_since(Instant::now());
   if remaining.is_zero(){return StopResult::StillInFlight}
   let Ok((next,_))=self.done.wait_timeout(state,remaining) else{return StopResult::StillInFlight};
   state=next;
  }
  StopResult::Drained
 }
 pub fn is_revoked(&self)->bool{self.state.lock().map_or(true,|s|s.revoked)}
}
impl Default for MemoryEgressFence{fn default()->Self{Self::new()}}
impl Drop for EgressLease<'_>{fn drop(&mut self){if self.active {if let Ok(mut s)=self.fence.state.lock(){s.active=s.active.saturating_sub(1);self.fence.done.notify_all();}}}}
#[cfg(test)]mod tests{use super::*;#[test]fn revoke_blocks_future(){let f=MemoryEgressFence::new();let x=f.admit(3).unwrap();assert_eq!(f.revoke(Duration::from_millis(1)),StopResult::StillInFlight);drop(x);assert_eq!(f.revoke(Duration::from_millis(1)),StopResult::Drained);assert!(f.admit(3).is_none());}}

impl Drop for OwnedEgressLease{
 fn drop(&mut self){
  if self.active {if let Ok(mut state)=self.fence.state.lock(){
   state.active=state.active.saturating_sub(1);
   self.fence.done.notify_all();
  }}
 }
}

#[cfg(test)]mod owned_lease_tests {
 use super::*;
 #[test]fn owned_lease_survives_scope_and_revision_updates(){
  let gate=std::sync::Arc::new(MemoryEgressFence::new());
  {let _lease=gate.admit_owned(5).unwrap();assert!(gate.admit_owned(6).is_none());}
  assert!(gate.admit_owned(6).is_some());
  assert!(gate.admit_owned(4).is_none());
 }
}
