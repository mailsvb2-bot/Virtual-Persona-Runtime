//! Authenticated, server-owned TTS provider switch. Secrets never enter HTTP JSON.
use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde::Deserialize;
use tiny_http::Request;

use super::{
    AppState, HttpResponse, error_response, json_response, parse_json, reject_if_session_ending,
    with_engine, with_engine_result,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TtsSelectBody {
    provider: String,
}

pub(super) fn snapshot(state: &AppState) -> HttpResponse {
    with_engine(state, |engine| json_response(200, &engine.tts_selection()))
}

pub(super) fn select(
    request: &mut Request,
    state: &Arc<AppState>,
) -> Result<HttpResponse, HttpResponse> {
    if state.voice_busy.load(Ordering::Acquire) {
        return Err(error_response(409, "VOICE_BUSY"));
    }
    let body = parse_json::<TtsSelectBody>(request)?;
    reject_if_session_ending(state)?;
    with_engine_result(state, |engine| {
        engine.select_tts_provider(&body.provider)?;
        Ok(json_response(200, &engine.tts_selection()))
    })
}
