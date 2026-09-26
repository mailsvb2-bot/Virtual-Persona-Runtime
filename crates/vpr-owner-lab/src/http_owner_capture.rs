use parking_lot::Mutex as ParkingMutex;
use serde::Deserialize;
use tiny_http::Request;
use vpr_capture::CaptureError;
use vpr_domain::{
    ClaimId, ClaimKind, ConstitutionBoundary, DerivationKind, OwnerClaim, OwnerClaimRecord,
    PersonaId, PersonaIdentity, PersonaMode, PersonaProfile, PersonaVersion, ProfileError,
    SourceKind, VerificationState,
};
use vpr_owner_lab::{OwnerCaptureError, OwnerContextState, Rt0OwnerCapture};

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

#[derive(Debug, Deserialize)]
struct ImportReviewedProfileBody {
    persona_id: String,
    claims: Vec<ImportReviewedClaimBody>,
}

#[derive(Debug, Deserialize)]
struct ImportReviewedClaimBody {
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
        "/api/persona/reviewed/import" => import_reviewed_profile(request, state),
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
    Ok(json_response(200, &engine.status()))
}

fn import_reviewed_profile(
    request: &mut Request,
    state: &AppState,
) -> Result<HttpResponse, HttpResponse> {
    let body = parse_json::<ImportReviewedProfileBody>(request)?;

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

    let profile = build_reviewed_profile(body).map_err(|_| error_response(400, "INVALID_INPUT"))?;
    engine
        .bind_reviewed_profile(profile)
        .map_err(|error| lab_error_response(&error))?;
    Ok(json_response(200, &engine.status()))
}

fn build_reviewed_profile(body: ImportReviewedProfileBody) -> Result<PersonaProfile, ()> {
    if body.claims.is_empty() {
        return Err(());
    }

    let persona_id = PersonaId::new(body.persona_id).map_err(|_| ())?;
    let version = PersonaVersion::new(1).ok_or(())?;
    let identity = PersonaIdentity::new(persona_id, version, PersonaMode::DigitalTwin);
    let mut profile = PersonaProfile::new(identity, ConstitutionBoundary::strict_digital_twin());
    let mut claim_ids = Vec::with_capacity(body.claims.len());

    for claim in body.claims {
        let claim_id = ClaimId::new(claim.claim_id).map_err(|_| ())?;
        let kind = parse_claim_kind(&claim.kind).ok_or(())?;
        let record = OwnerClaimRecord::capture(
            claim_id.clone(),
            OwnerClaim {
                statement: claim.statement,
                kind,
                source: SourceKind::Owner,
                verification: VerificationState::Unverified,
                derivation: DerivationKind::Direct,
            },
        )
        .map_err(|_| ())?;
        profile.add_captured_claim(record).map_err(|_| ())?;
        claim_ids.push(claim_id);
    }

    profile.mark_capture_complete().map_err(|_| ())?;
    for claim_id in &claim_ids {
        profile.approve_claim(claim_id).map_err(|_| ())?;
    }
    profile.approve_initial_review().map_err(|_| ())?;
    Ok(profile)
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
    capture
        .complete_initial_review()
        .map_err(capture_error_response)?;
    let reviewed = slot
        .take()
        .ok_or_else(|| error_response(500, "INTERNAL_ERROR"))?;
    engine
        .bind_reviewed_profile(reviewed.into_profile())
        .map_err(|error| lab_error_response(&error))?;
    Ok(json_response(200, &engine.status()))
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
    fn two_claim_legacy_import_builds_reviewed_profile_without_inventing_a_third_claim() {
        let body = ImportReviewedProfileBody {
            persona_id: "legacy-two-claim-owner".into(),
            claims: vec![
                ImportReviewedClaimBody {
                    claim_id: "identity-self-description".into(),
                    statement: "Сергей, предприниматель".into(),
                    kind: "factual".into(),
                },
                ImportReviewedClaimBody {
                    claim_id: "preference-communication-style".into(),
                    statement: "Кратко и по существу".into(),
                    kind: "preference".into(),
                },
            ],
        };

        let profile = build_reviewed_profile(body).expect("legacy import must be accepted");
        assert_eq!(profile.identity().version().get(), 2);
        assert_eq!(
            profile.capture_state(),
            vpr_domain::PersonaCaptureState::Reviewed
        );
        assert_eq!(profile.claims().len(), 2);
        assert!(
            profile
                .claims()
                .iter()
                .all(vpr_domain::OwnerClaimRecord::is_owner_reviewed)
        );
        assert!(
            profile
                .claim(&ClaimId::new("opinion-core-principle").unwrap())
                .is_none()
        );
    }

    #[test]
    fn claim_kind_parser_is_closed_over_canonical_values() {
        assert_eq!(parse_claim_kind("opinion"), Some(ClaimKind::Opinion));
        assert_eq!(parse_claim_kind("OPINION"), None);
        assert_eq!(parse_claim_kind("model_guess"), None);
    }
}
