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
    with_engine(state, |engine| {
        let mut selection = engine.tts_selection();
        if let Some(profile) = state.voice_profile.as_ref() {
            selection["voice_profile"] = super::voice_profile_public(profile);
        }
        json_response(200, &selection)
    })
}

pub(super) fn select(
    request: &mut Request,
    state: &Arc<AppState>,
) -> Result<HttpResponse, HttpResponse> {
    if state.rt0_evidence_mode {
        return Err(error_response(409, "TTS_LOCKED_IN_EVIDENCE_MODE"));
    }
    if state.voice_busy.load(Ordering::Acquire) {
        return Err(error_response(409, "VOICE_BUSY"));
    }
    if let Ok(engine) = state.engine.lock() {
        if !matches!(engine.status().session_state, "none" | "closed") {
            return Err(error_response(409, "TTS_LOCKED_DURING_SESSION"));
        }
    }
    let body = parse_json::<TtsSelectBody>(request)?;
    reject_if_session_ending(state)?;
    with_engine_result(state, |engine| {
        engine.select_tts_provider(&body.provider)?;
        Ok(json_response(200, &engine.tts_selection()))
    })
}
