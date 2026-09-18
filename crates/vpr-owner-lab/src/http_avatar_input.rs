use serde::Deserialize;
use tiny_http::Request;
use vpr_owner_lab::OwnerLabTurnInput;

use super::{AppState, HttpResponse, apply_input, parse_json};

#[derive(Deserialize)]
struct SpeakBody {
    text: String,
}

#[derive(Deserialize)]
struct AudioBody {
    audio_url: String,
}

pub(super) fn route_post(
    path: &str,
    request: &mut Request,
    state: &AppState,
) -> Option<Result<HttpResponse, HttpResponse>> {
    match path {
        "/api/avatar/speak" => Some(
            parse_json::<SpeakBody>(request)
                .and_then(|body| apply_input(state, OwnerLabTurnInput::Text(body.text))),
        ),
        "/api/avatar/audio" => Some(
            parse_json::<AudioBody>(request)
                .and_then(|body| apply_input(state, OwnerLabTurnInput::AudioUrl(body.audio_url))),
        ),
        _ => None,
    }
}
