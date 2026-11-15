//! Lifecycle for real browser TCP response registrations.
//! The writer handle is installed before the request reaches tiny_http; the
//! visitor session adopts it only after the canonical route returns its lease.
use super::Rt1HttpState;
use std::sync::Arc;
use vpr_owner_lab::{Rt1BrowserDeliveryGuard, Rt1VisitorConversation};
use vpr_runtime::TcpWriteRevocation;

pub(super) struct PendingDelivery {
    conversation: Arc<Rt1VisitorConversation>,
    _guard: Rt1BrowserDeliveryGuard,
    must_compensate: bool,
}

impl Rt1HttpState {
    pub(crate) fn register_transport_writer(
        &self,
        id: String,
        writer: TcpWriteRevocation,
    ) -> Result<(), String> {
        let mut writers = self.transport_writers.lock();
        if writers.len() >= 64 || writers.contains_key(&id) {
            writer.revoke();
            return Err("RT1_RESPONSE_WRITER_UNAVAILABLE".into());
        }
        writers.insert(id, writer);
        Ok(())
    }

    pub(crate) fn take_transport_writer(&self, id: &str) -> Option<TcpWriteRevocation> {
        self.transport_writers.lock().remove(id)
    }

    pub(crate) fn retain_transport_delivery(
        &self,
        id: String,
        conversation: Arc<Rt1VisitorConversation>,
        guard: Rt1BrowserDeliveryGuard,
        must_compensate: bool,
    ) -> bool {
        let mut pending = self.pending_deliveries.lock();
        if pending.len() >= 64 || pending.contains_key(&id) {
            return false;
        }
        pending.insert(
            id,
            PendingDelivery {
                conversation,
                _guard: guard,
                must_compensate,
            },
        );
        true
    }

    /// Called only after the outer TCP proxy finishes or abandons the client
    /// response. No registry lock is held while canonical revoke/STOP runs.
    pub(crate) fn finish_transport_delivery(&self, id: &str, delivered: bool) {
        self.transport_writers.lock().remove(id);
        let pending = self.pending_deliveries.lock().remove(id);
        if let Some(pending) = pending {
            if !delivered && pending.must_compensate {
                pending.conversation.revoke_undelivered_start();
            }
            drop(pending);
        }
    }
}
