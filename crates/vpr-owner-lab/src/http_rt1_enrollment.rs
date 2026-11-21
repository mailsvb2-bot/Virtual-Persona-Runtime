//! RT1 owner enrollment routes: one bearer per unfinished, reviewed capture.
//! Existing RT1 transport enforces local-only experiment mode and CSRF.
use serde::Deserialize;
use tiny_http::Request;
use vpr_owner_lab::Rt1OwnerController;

use super::{Rt1HttpResult, error_response, json_response, parse_json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Begin {
    persona_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrollmentToken {
    enrollment_token: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    enrollment_token: String,
    answer: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Approve {
    enrollment_token: String,
    claim_id: String,
}

pub(super) fn route(
    path: &str,
    request: &mut Request,
) -> Option<Rt1HttpResult> {
    let controller = Rt1OwnerController::new();
    let response = match path {
        "begin" => {
            let body: Begin = match parse_json(request) {
                Ok(body) => body,
                Err(response) => return Some(Err(response)),
            };
            controller
                .begin_owner_registration(body.persona_id)
                .map(|(enrollment_token, capture)| {
                    json_response(201, &serde_json::json!({
                        "enrollment_token": enrollment_token,
                        "capture": capture,
                    }))
                })
                .map_err(|_| error_response(409, "RT1_REGISTRATION_UNAVAILABLE"))
        }
        "status" => {
            let body: EnrollmentToken = match parse_json(request) {
                Ok(body) => body,
                Err(response) => return Some(Err(response)),
            };
            controller
                .owner_registration_status(&body.enrollment_token)
                .map(|capture| json_response(200, &capture))
                .map_err(|_| error_response(403, "RT1_REGISTRATION_DENIED"))
        }
        "answer" => {
            let body: Answer = match parse_json(request) {
                Ok(body) => body,
                Err(response) => return Some(Err(response)),
            };
            controller
                .owner_registration_answer(&body.enrollment_token, body.answer)
                .map(|capture| json_response(200, &capture))
                .map_err(|_| error_response(409, "RT1_CAPTURE_ANSWER_DENIED"))
        }
        "finish" => {
            let body: EnrollmentToken = match parse_json(request) {
                Ok(body) => body,
                Err(response) => return Some(Err(response)),
            };
            controller
                .owner_registration_finish(&body.enrollment_token)
                .map(|capture| json_response(200, &capture))
                .map_err(|_| error_response(409, "RT1_CAPTURE_FINISH_DENIED"))
        }
        "approve" => {
            let body: Approve = match parse_json(request) {
                Ok(body) => body,
                Err(response) => return Some(Err(response)),
            };
            controller
                .owner_registration_approve(&body.enrollment_token, &body.claim_id)
                .map(|capture| json_response(200, &capture))
                .map_err(|_| error_response(409, "RT1_CAPTURE_REVIEW_DENIED"))
        }
        "complete" => {
            let body: EnrollmentToken = match parse_json(request) {
                Ok(body) => body,
                Err(response) => return Some(Err(response)),
            };
            controller
                .complete_owner_registration(&body.enrollment_token)
                .map(|(owner_id, owner_token)| {
                    json_response(201, &serde_json::json!({
                        "owner_id": owner_id,
                        "owner_token": owner_token,
                        "publication_status": "private",
                    }))
                })
                .map_err(|_| error_response(409, "RT1_REGISTRATION_INCOMPLETE"))
        }
        _ => return None,
    };
    Some(response)
}
