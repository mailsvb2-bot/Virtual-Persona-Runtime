//! Exact RT1 HTTP extension: only the existing local RT1 router calls this.
//! `PostgreSQL` credentials are NEVER used as publication credentials. This
//! module requires an independently admitted, live RT1 visitor session.
use super::{AppState, HttpResponse, error_response, json_response, parse_json};
#[path = "rt1_memory_port.rs"]
mod port;
use port::VisitorMemoryPort;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;
use vpr_owner_lab::Rt1VisitorConversation;
use vpr_rt1_text_memory_postgres::{
    MemoryError, MemorySource, StablePrincipal, TextMemory,
    egress::{MemoryEgressFence, StopResult},
};

/// A single extension of the canonical HTTP route, not a second responder.
pub(super) fn route(
    path: &str,
    request: &mut tiny_http::Request,
    state: &Arc<AppState>,
) -> Option<super::Rt1RoutedResponse> {
    if path == "/api/rt1/owner/memory/admin" {
        let response = owner_admin(request).unwrap_or_else(|e| e);
        return Some(super::Rt1RoutedResponse {
            response,
            delivery: None,
        });
    }
    if path.starts_with("/api/rt1/visitor/memory/") {
        let result = handle(path, request, state);
        if matches!(
            path,
            "/api/rt1/visitor/memory/consent"
                | "/api/rt1/visitor/memory/erase"
                | "/api/rt1/visitor/memory/revoke"
                | "/api/rt1/visitor/memory/rotate"
        ) {
            // A revoked session must still receive its privacy-operation receipt.
            return Some(super::Rt1RoutedResponse {
                response: result.map_or_else(|response| response, |(response, _)| response),
                delivery: None,
            });
        }
        return Some(super::guarded_reply(result));
    }
    None
}

static STORE: OnceLock<Arc<TextMemory>> = OnceLock::new();
static STORE_INIT_LOCK: Mutex<()> = Mutex::new(());
static EGRESS_REGISTRY: OnceLock<Mutex<HashMap<[u8; 32], Weak<MemoryEgressFence>>>> =
    OnceLock::new();
fn fingerprint(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}
fn egress_for_token(token: &str) -> Result<Arc<MemoryEgressFence>, HttpResponse> {
    let guard = EGRESS_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = guard
        .lock()
        .map_err(|_| error_response(503, "RT1_MEMORY_FENCE_UNAVAILABLE"))?;
    map.retain(|_, value| value.strong_count() > 0);
    let key = fingerprint(token);
    if let Some(existing) = map.get(&key).and_then(Weak::upgrade)
        && !existing.is_revoked()
    {
        return Ok(existing);
    }
    if map.len() >= 512 {
        return Err(error_response(503, "RT1_MEMORY_FENCE_CAPACITY"));
    }
    let created = Arc::new(MemoryEgressFence::new());
    map.insert(key, Arc::downgrade(&created));
    Ok(created)
}
fn revoke_egress_for_token(token: &str) -> bool {
    let Some(registry) = EGRESS_REGISTRY.get() else {
        return true;
    };
    let fence = registry
        .lock()
        .ok()
        .and_then(|m| m.get(&fingerprint(token)).and_then(Weak::upgrade));
    fence.is_none_or(|f| f.revoke(Duration::from_millis(500)) == StopResult::Drained)
}

fn store() -> Result<Arc<TextMemory>, HttpResponse> {
    if let Some(db) = STORE.get() {
        return Ok(Arc::clone(db));
    }
    let _startup_guard = STORE_INIT_LOCK
        .lock()
        .map_err(|_| error_response(503, "RT1_MEMORY_DB_UNAVAILABLE"))?;
    // Exactly one connection may acquire the exclusive owner lease.
    if let Some(db) = STORE.get() {
        return Ok(Arc::clone(db));
    }
    let dsn = std::env::var("VPR_RT1_MEMORY_POSTGRES_DSN")
        .map_err(|_| error_response(503, "RT1_MEMORY_DISABLED"))?;
    let db = Arc::new(
        TextMemory::connect_exclusive(&dsn)
            .map_err(|_| error_response(503, "RT1_MEMORY_DB_UNAVAILABLE"))?,
    );
    db.check_schema()
        .map_err(|_| error_response(503, "RT1_MEMORY_SCHEMA_UNAVAILABLE"))?;
    STORE
        .set(Arc::clone(&db))
        .map_err(|_| error_response(503, "RT1_MEMORY_DB_UNAVAILABLE"))?;
    let worker = Arc::downgrade(&db);
    std::thread::Builder::new()
        .name("vpr-memory-maintenance".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(2));
                let Some(store) = worker.upgrade() else { break };
                let _ = store.drain_due_outbox(8);
                let _ = store.purge_memory_expired(32);
            }
        })
        .map_err(|_| error_response(503, "RT1_MEMORY_WORKER_UNAVAILABLE"))?;
    Ok(db)
}
fn workspace_id() -> Result<String, HttpResponse> {
    let value = std::env::var("VPR_RT1_MEMORY_WORKSPACE_ID")
        .map_err(|_| error_response(503, "RT1_MEMORY_WORKSPACE_UNCONFIGURED"))?;
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(error_response(503, "RT1_MEMORY_WORKSPACE_UNCONFIGURED"));
    }
    Ok(value)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryRequest {
    session_id: String,
    #[serde(default)]
    memory_token: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    operation_id: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    source_digest: Option<String>,
    #[serde(default)]
    valid_from: Option<i64>,
    #[serde(default)]
    valid_until: Option<i64>,
}
fn denied(_: MemoryError) -> HttpResponse {
    error_response(403, "RT1_MEMORY_DENIED")
}

/// Privacy fence: revoke the existing canonical visitor execution path for
/// *every* active binding of this personal memory credential before changing
/// consent, rotating or destroying its durable state. Copy Arc handles out of
/// the registry: remote STOP can block and must never hold the registry mutex.
/// A STOP failure is returned after the DB mutation but every matching local
/// session is already revoked and cannot initiate another provider call.
fn stop_bound_memory_sessions(state: &AppState, token: &str) -> bool {
    let active: Vec<_> = state
        .rt1
        .conversations
        .lock()
        .iter()
        .map(|(id, conversation)| (id.clone(), Arc::clone(conversation)))
        .collect();
    let matches: Vec<_> = active
        .into_iter()
        .filter(|(_, session)| session.memory_credential_matches(token))
        .collect();
    // Revoke future LLM dispatch BEFORE requesting canonical remote STOP.
    let mut confirmed = revoke_egress_for_token(token);
    for (id, conversation) in matches {
        let stopped = conversation.revoke_memory_session();
        if stopped.is_err()
            || super::finalize_registered_stop(
                &state.rt1.conversations,
                &id,
                &conversation,
                stopped,
                conversation.has_remote_video_stop_receipt_path(),
            )
            .is_err()
        {
            confirmed = false;
        }
    }
    confirmed
}

fn session(state: &AppState, id: &str) -> Result<Arc<Rt1VisitorConversation>, HttpResponse> {
    let c = super::lookup(state, id)?;
    if !c.is_active() {
        return Err(error_response(409, "RT1_MEMORY_SESSION_REVOKED"));
    }
    Ok(c)
}
fn scope(
    memory: &TextMemory,
    token: &str,
    workspace: &str,
    persona: &str,
) -> Result<StablePrincipal, HttpResponse> {
    let principal = memory.authenticate(token).map_err(denied)?;
    if !principal.matches_scope(workspace, persona) {
        return Err(error_response(403, "RT1_MEMORY_SCOPE_DENIED"));
    }
    Ok(principal)
}

struct MemoryActionContext<'a> {
    state: &'a AppState,
    db: &'a Arc<TextMemory>,
    conversation: &'a Arc<Rt1VisitorConversation>,
    principal: &'a StablePrincipal,
    token: &'a str,
    body: &'a MemoryRequest,
}

fn handle_note_action(
    path: &str,
    ctx: &MemoryActionContext<'_>,
) -> Result<serde_json::Value, HttpResponse> {
    let (db, principal, token, body, conversation) =
        (ctx.db, ctx.principal, ctx.token, ctx.body, ctx.conversation);
    let result = match path {
        "/api/rt1/visitor/memory/connect" => {
            conversation
                .bind_memory(Arc::new(VisitorMemoryPort::create(
                    db,
                    principal.clone(),
                    token,
                )?))
                .map_err(|_| error_response(409, "RT1_MEMORY_BIND_DENIED"))?;
            serde_json::json!({"ok":true, "memory_connected":true})
        }
        "/api/rt1/visitor/memory/list" => {
            let facts = db
                .list_facts(&principal, body.cursor.as_deref(), 50)
                .map_err(denied)?;
            let next_cursor = if facts.len() == 50 {
                facts.last().map(|fact| fact.key.clone())
            } else {
                None
            };
            serde_json::json!({"facts":facts,"next_cursor":next_cursor})
        }
        "/api/rt1/visitor/memory/remember" => {
            let key = body
                .key
                .as_deref()
                .ok_or_else(|| error_response(400, "RT1_MEMORY_INVALID_INPUT"))?;
            let content = body
                .content
                .as_deref()
                .ok_or_else(|| error_response(400, "RT1_MEMORY_INVALID_INPUT"))?;
            let id = body
                .operation_id
                .as_deref()
                .ok_or_else(|| error_response(400, "RT1_MEMORY_INVALID_INPUT"))?;
            db.remember(
                &principal,
                id,
                key,
                content,
                MemorySource::VisitorExplicit,
                365,
            )
            .map_err(denied)?;
            serde_json::json!({"ok":true,"saved":true})
        }
        "/api/rt1/visitor/memory/contradictions" => {
            let content = body
                .content
                .as_deref()
                .ok_or_else(|| error_response(400, "RT1_MEMORY_INVALID_INPUT"))?;
            let matches = db
                .possible_contradictions(&principal, content)
                .map_err(denied)?;
            serde_json::json!({"candidates":matches,"automatic_overwrite":false})
        }
        "/api/rt1/visitor/memory/validity" => {
            let key = body
                .key
                .as_deref()
                .ok_or_else(|| error_response(400, "RT1_MEMORY_INVALID_INPUT"))?;
            db.set_validity(&principal, key, body.valid_from, body.valid_until)
                .map_err(denied)?;
            serde_json::json!({"ok":true,"validity_updated":true})
        }
        "/api/rt1/visitor/memory/forget" => {
            let key = body
                .key
                .as_deref()
                .ok_or_else(|| error_response(400, "RT1_MEMORY_INVALID_INPUT"))?;
            let id = body
                .operation_id
                .as_deref()
                .ok_or_else(|| error_response(400, "RT1_MEMORY_INVALID_INPUT"))?;
            db.forget(&principal, id, key).map_err(denied)?;
            serde_json::json!({"ok":true,"forgotten":true})
        }
        _ => return Err(error_response(404, "RT1_MEMORY_ROUTE_NOT_FOUND")),
    };
    Ok(result)
}

fn handle_summary_action(
    path: &str,
    ctx: &MemoryActionContext<'_>,
) -> Result<serde_json::Value, HttpResponse> {
    let (db, principal, body) = (ctx.db, ctx.principal, ctx.body);
    let result = match path {
        "/api/rt1/visitor/memory/summary/draft" => {
            let (text, source_digest) = db.draft_fact_summary(&principal).map_err(denied)?;
            serde_json::json!({"text":text,"source_digest":source_digest,"approved":false})
        }
        "/api/rt1/visitor/memory/summary/approve" => {
            let key = body
                .key
                .as_deref()
                .ok_or_else(|| error_response(400, "RT1_MEMORY_INVALID_INPUT"))?;
            let content = body
                .content
                .as_deref()
                .ok_or_else(|| error_response(400, "RT1_MEMORY_INVALID_INPUT"))?;
            let digest = body
                .source_digest
                .as_deref()
                .ok_or_else(|| error_response(400, "RT1_MEMORY_INVALID_INPUT"))?;
            // Verify candidate still describes current facts. The user must
            // explicitly approve the draft after inspecting it.
            let (_current_text, current_digest) =
                db.draft_fact_summary(&principal).map_err(denied)?;
            if digest != current_digest {
                return Err(error_response(409, "RT1_MEMORY_SUMMARY_STALE"));
            }
            db.save_approved_summary(&principal, key, content, digest)
                .map_err(denied)?;
            serde_json::json!({"ok":true,"approved":true})
        }
        "/api/rt1/visitor/memory/summary/list" => {
            let summaries = db.list_summaries(&principal).map_err(denied)?;
            serde_json::json!({"summaries":summaries})
        }
        _ => return Err(error_response(404, "RT1_MEMORY_ROUTE_NOT_FOUND")),
    };
    Ok(result)
}

fn handle_privacy_action(
    path: &str,
    ctx: &MemoryActionContext<'_>,
) -> Result<(HttpResponse, Arc<Rt1VisitorConversation>), HttpResponse> {
    let (db, principal, token, body, state) =
        (ctx.db, ctx.principal, ctx.token, ctx.body, ctx.state);
    let conversation = Arc::clone(ctx.conversation);
    match path {
        "/api/rt1/visitor/memory/consent" => {
            let enabled = body
                .enabled
                .ok_or_else(|| error_response(400, "RT1_MEMORY_INVALID_INPUT"))?;
            if !enabled {
                let stopped = stop_bound_memory_sessions(state, token);
                let revision = db.set_consent(&principal, false).map_err(denied)?;
                return if stopped {
                    Ok((
                        json_response(
                            200,
                            &serde_json::json!({"ok":true,
                        "consent":false,"revision":revision,"session_stopped":true}),
                        ),
                        conversation,
                    ))
                } else {
                    Err(error_response(502, "RT1_MEMORY_REMOTE_STOP_UNCONFIRMED"))
                };
            }
            let revision = db.set_consent(&principal, true).map_err(denied)?;
            conversation
                .authorize_memory_action()
                .map_err(|_| error_response(409, "RT1_VISITOR_REVOKED"))?;
            Ok((
                json_response(200, &serde_json::json!({"ok":true,"consent":true,"revision":revision})),
                conversation,
            ))
        }
        "/api/rt1/visitor/memory/erase" => {
            let stopped = stop_bound_memory_sessions(state, token);
            db.erase_visitor(&principal).map_err(denied)?;
            return if stopped {
                Ok((
                    json_response(
                        200,
                        &serde_json::json!({"ok":true,"erased":true,
                    "session_stopped":true}),
                    ),
                    conversation,
                ))
            } else {
                Err(error_response(502, "RT1_MEMORY_REMOTE_STOP_UNCONFIRMED"))
            };
        }
        "/api/rt1/visitor/memory/rotate" => {
            let stopped = stop_bound_memory_sessions(state, token);
            if !stopped {
                return Err(error_response(502, "RT1_MEMORY_REMOTE_STOP_UNCONFIRMED"));
            }
            let replacement = db.rotate_credential(token).map_err(denied)?;
            // The canonical STOP has revoked this conversation. Return the
            // replacement credential directly: a post-STOP session authorize
            // would fail and irretrievably hide the newly issued secret.
            return Ok((
                json_response(
                    200,
                    &serde_json::json!({
                        "ok":true,"memory_token":replacement,"session_stopped":true
                    }),
                ),
                conversation,
            ));
        }
        "/api/rt1/visitor/memory/revoke" => {
            let stopped = stop_bound_memory_sessions(state, token);
            db.revoke_credential(token).map_err(denied)?;
            return if stopped {
                Ok((
                    json_response(
                        200,
                        &serde_json::json!({"ok":true,"revoked":true,
                    "session_stopped":true}),
                    ),
                    conversation,
                ))
            } else {
                Err(error_response(502, "RT1_MEMORY_REMOTE_STOP_UNCONFIRMED"))
            };
        }
        _ => Err(error_response(404, "RT1_MEMORY_ROUTE_NOT_FOUND")),
    }
}

/// A tiny read-only context adapter. Always uses the SAME canonical `OwnerLab`
/// engine and provider turn. Database failure yields no private context.
/// Memory facts are data, not new instructions or verified Persona claims.
/// Called ONLY via current RT1 POST router; existing CSRF and loopback gates
/// remain in effect. Session is checked before and after each operation.
pub(super) fn handle(
    path: &str,
    request: &mut tiny_http::Request,
    state: &AppState,
) -> Result<(HttpResponse, Arc<Rt1VisitorConversation>), HttpResponse> {
    let body: MemoryRequest = parse_json(request)?;
    let conversation = session(state, &body.session_id)?;
    let persona = conversation
        .memory_persona_id()
        .map_err(|_| error_response(409, "RT1_MEMORY_PERSONA_DENIED"))?;
    let workspace = workspace_id()?;
    let db = store()?;
    if path == "/api/rt1/visitor/memory/receipt" {
        let token = body
            .memory_token
            .as_deref()
            .ok_or_else(|| error_response(403, "RT1_MEMORY_CREDENTIAL_REQUIRED"))?;
        let _principal = scope(&db, token, &workspace, &persona)?;
        return Ok((
            json_response(
                200,
                &serde_json::json!({
                    "last_explicit_note": conversation.memory_save_receipt_for(token)
                }),
            ),
            conversation,
        ));
    }
    if path == "/api/rt1/visitor/memory/enroll" {
        // A new identity is minted only within a real authorized visitor
        // session. Publication invitation alone never authenticates memory.
        let credential = db.enroll_authorized(&workspace, &persona).map_err(denied)?;
        if conversation.authorize_memory_action().is_err() {
            let _ = db.revoke_credential(&credential.token);
            return Err(error_response(409, "RT1_VISITOR_REVOKED"));
        }
        return Ok((
            json_response(
                200,
                &serde_json::json!({
                    "memory_token": credential.token,
                    "message": "Save this personal key securely; it is shown only once"
                }),
            ),
            conversation,
        ));
    }
    let token = body
        .memory_token
        .as_deref()
        .ok_or_else(|| error_response(403, "RT1_MEMORY_CREDENTIAL_REQUIRED"))?;
    let principal = scope(&db, token, &workspace, &persona)?;
    let context = MemoryActionContext {
        state,
        db: &db,
        conversation: &conversation,
        principal: &principal,
        token,
        body: &body,
    };
    if matches!(
        path,
        "/api/rt1/visitor/memory/consent"
            | "/api/rt1/visitor/memory/erase"
            | "/api/rt1/visitor/memory/rotate"
            | "/api/rt1/visitor/memory/revoke"
    ) {
        return handle_privacy_action(path, &context);
    }
    let result = if path.starts_with("/api/rt1/visitor/memory/summary/") {
        handle_summary_action(path, &context)?
    } else {
        handle_note_action(path, &context)?
    };
    conversation
        .authorize_memory_action()
        .map_err(|_| error_response(409, "RT1_VISITOR_REVOKED"))?;
    Ok((json_response(200, &result), conversation))
}

/// Authorized memory binding for a NEW visitor conversation, BEFORE provider CREATE.
/// Never construct a second conversation engine or bypass Persona authorization.
pub(super) fn bind_on_start(
    engine: &mut vpr_owner_lab::OwnerLabEngine,
    token: &str,
) -> Result<(), HttpResponse> {
    let db = store()?;
    let workspace = workspace_id()?;
    let persona = engine
        .memory_persona_id()
        .map_err(|_| error_response(403, "RT1_MEMORY_PERSONA_MISSING"))?;
    let principal = scope(&db, token, &workspace, &persona)?;
    let port = VisitorMemoryPort::create(&db, principal, token)?;
    engine
        .bind_memory_port(Arc::new(port))
        .map_err(|_| error_response(409, "RT1_MEMORY_BIND_DENIED"))
}

/// Owner-only administrative metadata. Never returns visitor identifiers or memory text.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryAdminRequest {
    owner_token: String,
}

pub(super) fn owner_admin(request: &mut tiny_http::Request) -> Result<HttpResponse, HttpResponse> {
    let body: MemoryAdminRequest = parse_json(request)?;
    // Check canonical owner authority BEFORE initializing or querying memory DB.
    super::Rt1OwnerController::new()
        .status(&body.owner_token)
        .map_err(|_| error_response(403, "RT1_MEMORY_OWNER_DENIED"))?;
    let aggregate = store()?
        .admin_snapshot()
        .map_err(|_| error_response(503, "RT1_MEMORY_ADMIN_UNAVAILABLE"))?;
    Ok(json_response(
        200,
        &serde_json::json!({
            "active_subjects": aggregate.active_subjects,
            "facts": aggregate.facts,
            "queued_writes": aggregate.queued_writes,
            "expired_facts": aggregate.expired_facts,
            "unreconciled_erasures": aggregate.unreconciled_erasure_requests
        }),
    ))
}
