//! Lifecycle for real browser TCP response registrations.
//! The writer handle is installed before the request reaches `tiny_http`; the
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


pub(super) fn respond_post(mut request: tiny_http::Request, state: &Arc<super::AppState>) {
    let transport_id = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("X-VPR-RT1-Transport-Id"))
        .map(|header| header.value.as_str().to_owned());
    if !super::super::valid_post_headers(&request, &state.csrf_token, state.port) {
        let _ = request.respond(super::error_response(403, "CSRF_DENIED"));
        return;
    }
    if !super::enabled() {
        let _ = request.respond(super::error_response(404, "NOT_FOUND"));
        return;
    }
    let path = request.url().to_owned();
    let Some(routed) = super::route_post(&path, &mut request, state) else {
        let _ = request.respond(super::error_response(404, "NOT_FOUND"));
        return;
    };
    if let Some(conversation) = routed.delivery {
        let must_compensate = path == "/api/rt1/visitor/start"
            || path == "/api/rt1/visitor/answer"
            || path == "/api/rt1/visitor/ice"
            || path == "/api/rt1/visitor/text"
            || path == "/api/rt1/visitor/voice";
        if let Some(transport_id) = transport_id.as_deref() {
            let Some(writer) = state.rt1.take_transport_writer(transport_id) else {
                conversation.revoke_undelivered_start();
                let response = super::delivery_denied_after_revoke(
                    conversation.has_remote_video_stop_receipt_path(),
                    conversation.remote_media_stop_confirmed(),
                );
                let _ = request.respond(response);
                return;
            };
            let abort = writer.clone();
            if let Ok(guard) = conversation.begin_browser_tcp_delivery(writer) {
                if !state.rt1.retain_transport_delivery(
                    transport_id.to_owned(),
                    Arc::clone(&conversation),
                    guard,
                    must_compensate,
                ) {
                    abort.revoke();
                    conversation.revoke_undelivered_start();
                    let _ = request.respond(super::error_response(
                        503,
                        "RT1_RESPONSE_WRITER_UNAVAILABLE",
                    ));
                    return;
                }
                if request.respond(routed.response).is_err() {
                    state.rt1.finish_transport_delivery(transport_id, false);
                }
            } else {
                abort.revoke();
                conversation.revoke_undelivered_start();
                let response = super::delivery_denied_after_revoke(
                    conversation.has_remote_video_stop_receipt_path(),
                    conversation.remote_media_stop_confirmed(),
                );
                let _ = request.respond(response);
            }
            return;
        }
        // Direct tiny_http test requests have no outer socket registration.
        // Production browser traffic always uses the cancellable public listener.
        if let Ok(_lease) = conversation.begin_browser_delivery() {
            if request.respond(routed.response).is_err() && must_compensate {
                conversation.revoke_undelivered_start();
            }
        } else if must_compensate {
            conversation.revoke_undelivered_start();
            let response = super::delivery_denied_after_revoke(
                conversation.has_remote_video_stop_receipt_path(),
                conversation.remote_media_stop_confirmed(),
            );
            let _ = request.respond(response);
        } else {
            let _ = request.respond(super::error_response(409, "RT1_VISITOR_DELIVERY_DENIED"));
        }
    } else {
        let _ = request.respond(routed.response);
    }
}
