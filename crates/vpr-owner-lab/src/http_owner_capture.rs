use parking_lot::Mutex as ParkingMutex;
use serde::Deserialize;
use tiny_http::Request;
use vpr_capture::CaptureError;
use vpr_domain::{ClaimId, ClaimKind, PersonaId, ProfileError};
use vpr_owner_lab::{
    OwnerCaptureError, OwnerContextState, ReviewedOwnerClaimSnapshot,
    ReviewedOwnerContextSnapshot, Rt0OwnerCapture, save_reviewed_persona,
};

use crate::{
    AppState, HttpResponse, error_response, json_response, lab_error_response, parse_empty_json,
    parse_json,
};

#[derive(Default)]
pub(crate) struct OwnerCaptureHttpState {
    capture: ParkingMutex<Option<Rt0OwnerCapture>>,
}

#[derive(Debug, Deserialize)]
struct CreatePersonaBody {
    persona_id: String,
}

#[derive(Debug, Deserialize)]
struct CaptureAnswerBody {
    answer: String,
}

#[derive(Debug, Deserialize)]
struct ClaimBody {
    claim_id: String,
}

#[derive(Debug, Deserialize)]
struct CorrectClaimBody {
    claim_id: String,
    statement: String,
    kind: String,
}

pub(crate) fn snapshot(state: &AppState) -> HttpResponse {
    let capture_snapshot = state
        .owner_capture
        .capture
        .lock()
        .as_ref()
        .map(Rt0OwnerCapture::snapshot);
    if let Some(snapshot) = capture_snapshot {
        return json_response(200, &snapshot);
    }

    let Ok(engine) = state.engine.lock() else {
        return error_response(500, "INTERNAL_ERROR");
    };
    let status = engine.status();
    json_response(
        200,
        &serde_json::json!({
            "capture": null,
            "owner_context_state": status.owner_context_state,
            "persona_version": status.persona_version,
            "reviewed_owner_claims": status.reviewed_owner_claims,
        }),
    )
}

pub(crate) fn route_post(
    path: &str,
    request: &mut Request,
    state: &AppState,
) -> Option<Result<HttpResponse, HttpResponse>> {
    let result = match path {
        "/api/persona/create" => create_persona(request, state),
        "/api/persona/capture/answer" => submit_answer(request, state),
        "/api/persona/capture/finish" => finish_capture(request, state),
        "/api/persona/claims/approve" => approve_claim(request, state),
        "/api/persona/claims/correct" => correct_claim(request, state),
        "/api/persona/review/complete" => complete_review(request, state),
        "/api/persona/reviewed" => reviewed_snapshot(request, state),
        _ => return None,
    };
    Some(result)
}

fn create_persona(request: &mut Request, state: &AppState) -> Result<HttpResponse, HttpResponse> {
    let body = parse_json::<CreatePersonaBody>(request)?;
    let persona_id =
        PersonaId::new(body.persona_id).map_err(|_| error_response(400, "INVALID_INPUT"))?;
    let engine = state
        .engine
        .lock()
        .map_err(|_| error_response(500, "INTERNAL_ERROR"))?;
    let status = engine.status();
    if status.owner_context_state != OwnerContextState::Missing
        || !matches!(status.session_state.as_str(), "none" | "closed")
    {
        return Err(error_response(409, "INVALID_STATE_TRANSITION"));
    }
    let mut slot = state.owner_capture.capture.lock();
    if slot.is_some() {
        return Err(error_response(409, "INVALID_STATE_TRANSITION"));
    }
    let capture = Rt0OwnerCapture::new(persona_id).map_err(capture_error_response)?;
    let snapshot = capture.snapshot();
    *slot = Some(capture);
    Ok(json_response(201, &snapshot))
}

fn submit_answer(request: &mut Request, state: &AppState) -> Result<HttpResponse, HttpResponse> {
    let body = parse_json::<CaptureAnswerBody>(request)?;
    with_capture(state, |capture| {
        capture.submit_answer(body.answer)?;
        Ok(capture.snapshot())
    })
    .map(|snapshot| json_response(200, &snapshot))
}

fn finish_capture(request: &mut Request, state: &AppState) -> Result<HttpResponse, HttpResponse> {
    parse_empty_json(request)?;
    with_capture(state, |capture| {
        capture.finish_capture()?;
        Ok(capture.snapshot())
    })
    .map(|snapshot| json_response(200, &snapshot))
}

fn approve_claim(request: &mut Request, state: &AppState) -> Result<HttpResponse, HttpResponse> {
    let body = parse_json::<ClaimBody>(request)?;
    let id = ClaimId::new(body.claim_id).map_err(|_| error_response(400, "INVALID_INPUT"))?;
    with_capture(state, |capture| {
        capture.approve_claim(&id)?;
        Ok(capture.snapshot())
    })
    .map(|snapshot| json_response(200, &snapshot))
}

fn correct_claim(request: &mut Request, state: &AppState) -> Result<HttpResponse, HttpResponse> {
    let body = parse_json::<CorrectClaimBody>(request)?;
    let id = ClaimId::new(body.claim_id).map_err(|_| error_response(400, "INVALID_INPUT"))?;
    let kind = parse_claim_kind(&body.kind).ok_or_else(|| error_response(400, "INVALID_INPUT"))?;

    {
        let mut slot = state.owner_capture.capture.lock();
        if let Some(capture) = slot.as_mut() {
            capture
                .correct_claim(&id, body.statement, kind)
                .map_err(capture_error_response)?;
            return Ok(json_response(200, &capture.snapshot()));
        }
    }

    let mut engine = state
        .engine
        .lock()
        .map_err(|_| error_response(500, "INTERNAL_ERROR"))?;
    engine
        .correct_owner_claim(&id, body.statement, kind)
        .map_err(|error| lab_error_response(&error))?;
    persist_reviewed_persona(&engine)?;
    Ok(json_response(200, &engine.status()))
}

fn reviewed_snapshot(
    request: &mut Request,
    state: &AppState,
) -> Result<HttpResponse, HttpResponse> {
    parse_empty_json(request)?;
    let engine = state
        .engine
        .lock()
        .map_err(|_| error_response(500, "INTERNAL_ERROR"))?;
    let snapshot = engine
        .reviewed_owner_context_snapshot()
        .map_err(|error| lab_error_response(&error))?;
    Ok(json_response(200, &snapshot))
}

fn complete_review(request: &mut Request, state: &AppState) -> Result<HttpResponse, HttpResponse> {
    parse_empty_json(request)?;
    let mut engine = state
        .engine
        .lock()
        .map_err(|_| error_response(500, "INTERNAL_ERROR"))?;
    let status = engine.status();
    if status.owner_context_state != OwnerContextState::Missing
        || !matches!(status.session_state.as_str(), "none" | "closed")
    {
        return Err(error_response(409, "INVALID_STATE_TRANSITION"));
    }

    let mut slot = state.owner_capture.capture.lock();
    let capture = slot
        .as_mut()
        .ok_or_else(|| error_response(409, "INVALID_STATE_TRANSITION"))?;

    if capture.snapshot().capture_state != "reviewed" {
        capture
            .complete_initial_review()
            .map_err(capture_error_response)?;
    }

    let reviewed_snapshot = reviewed_snapshot_from_capture(capture)?;
    save_reviewed_persona(&reviewed_snapshot)
        .map_err(|_| error_response(500, "PERSONA_PERSISTENCE_FAILED"))?;

    let reviewed = slot
        .take()
        .ok_or_else(|| error_response(500, "INTERNAL_ERROR"))?;
    engine
        .bind_reviewed_profile(reviewed.into_profile())
        .map_err(|error| lab_error_response(&error))?;
    Ok(json_response(200, &engine.status()))
}

fn reviewed_snapshot_from_capture(
    capture: &Rt0OwnerCapture,
) -> Result<ReviewedOwnerContextSnapshot, HttpResponse> {
    let snapshot = capture.snapshot();
    if snapshot.capture_state != "reviewed" || snapshot.claims.is_empty() {
        return Err(error_response(409, "INVALID_STATE_TRANSITION"));
    }

    let claims = snapshot
        .claims
        .into_iter()
        .map(|claim| {
            if !claim.owner_reviewed {
                return Err(error_response(409, "INVALID_STATE_TRANSITION"));
            }
            Ok(ReviewedOwnerClaimSnapshot {
                claim_id: claim.claim_id,
                statement: claim.statement,
                kind: claim.kind,
                revision: claim.revision,
            })
        })
        .collect::<Result<Vec<_>, HttpResponse>>()?;

    Ok(ReviewedOwnerContextSnapshot {
        persona_id: snapshot.persona_id,
        persona_version: snapshot.persona_version,
        claims,
    })
}

fn persist_reviewed_persona(engine: &vpr_owner_lab::OwnerLabEngine) -> Result<(), HttpResponse> {
    let snapshot = engine
        .reviewed_owner_context_snapshot()
        .map_err(|error| lab_error_response(&error))?;
    save_reviewed_persona(&snapshot).map_err(|_| error_response(500, "PERSONA_PERSISTENCE_FAILED"))
}

fn with_capture<T>(
    state: &AppState,
    operation: impl FnOnce(&mut Rt0OwnerCapture) -> Result<T, OwnerCaptureError>,
) -> Result<T, HttpResponse> {
    let mut slot = state.owner_capture.capture.lock();
    let capture = slot
        .as_mut()
        .ok_or_else(|| error_response(409, "INVALID_STATE_TRANSITION"))?;
    operation(capture).map_err(capture_error_response)
}

fn capture_error_response(error: OwnerCaptureError) -> HttpResponse {
    match error {
        OwnerCaptureError::Capture(
            CaptureError::BlankAnswer | CaptureError::Profile(ProfileError::BlankClaimStatement),
        ) => error_response(400, "INVALID_INPUT"),
        OwnerCaptureError::Capture(CaptureError::Profile(ProfileError::ClaimNotFound)) => {
            error_response(404, "CLAIM_NOT_FOUND")
        }
        OwnerCaptureError::Capture(_) => error_response(409, "INVALID_STATE_TRANSITION"),
        OwnerCaptureError::Plan(_)
        | OwnerCaptureError::InvalidStaticPlan
        | OwnerCaptureError::VersionUnavailable => error_response(500, "INTERNAL_ERROR"),
    }
}

fn parse_claim_kind(value: &str) -> Option<ClaimKind> {
    match value {
        "factual" => Some(ClaimKind::Factual),
        "opinion" => Some(ClaimKind::Opinion),
        "preference" => Some(ClaimKind::Preference),
        "prediction" => Some(ClaimKind::Prediction),
        "value_judgment" => Some(ClaimKind::ValueJudgment),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_kind_parser_is_closed_over_canonical_values() {
        assert_eq!(parse_claim_kind("opinion"), Some(ClaimKind::Opinion));
        assert_eq!(parse_claim_kind("OPINION"), None);
        assert_eq!(parse_claim_kind("model_guess"), None);
    }
}
