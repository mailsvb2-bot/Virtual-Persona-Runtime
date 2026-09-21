use serde::Deserialize;
use tiny_http::Request;

use super::{AppState, HttpResponse, parse_json, reject_if_session_ending, with_engine_result};

#[derive(Deserialize)]
struct ClientInterruptBody {
    playback_id: Option<String>,
}

#[derive(Deserialize)]
struct ClientEventBody {
    message: String,
}

#[derive(Deserialize)]
struct ClientDeliverySentBody {
    evidence_turn_sequence: u64,
    evidence_output_sequence: u64,
}

pub(super) fn route_post(
    path: &str,
    request: &mut Request,
    state: &AppState,
) -> Option<Result<HttpResponse, HttpResponse>> {
    match path {
        "/api/avatar/client-event" => {
            Some(parse_json::<ClientEventBody>(request).and_then(|body| {
                reject_if_session_ending(state)?;
                with_engine_result(state, |engine| {
                    engine
                        .parse_client_event(&body.message)
                        .map(|event| super::json_response(200, &event))
                })
            }))
        }
        "/api/avatar/client-interrupt" => {
            Some(parse_json::<ClientInterruptBody>(request).and_then(|body| {
                reject_if_session_ending(state)?;
                with_engine_result(state, |engine| {
                    engine
                        .prepare_client_interrupt(body.playback_id.as_deref())
                        .map(|command| super::json_response(200, &command))
                })
            }))
        }
        "/api/avatar/client-delivery-sent" => Some(
            parse_json::<ClientDeliverySentBody>(request).and_then(|body| {
                reject_if_session_ending(state)?;
                with_engine_result(state, |engine| {
                    engine
                        .acknowledge_voice_delivery_sent(
                            body.evidence_turn_sequence,
                            body.evidence_output_sequence,
                        )
                        .map(|()| super::json_response(200, &serde_json::json!({"ok": true})))
                })
            }),
        ),
        _ => None,
    }
}
