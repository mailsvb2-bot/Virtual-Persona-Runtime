use std::sync::atomic::Ordering;

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

const MAX_RESUMES_PER_REPLY: u8 = 4;

#[derive(Clone)]
pub(super) struct AuthorizedReply {
    pub request_sequence: u64,
    pub reply: String,
    pub resume_count: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResumeAnswerBody {
    request_sequence: u64,
    sentence_index: usize,
}

/// Split at natural sentence boundaries without letting browser-provided text
/// become the authoritative speech payload.
fn authorized_sentence_suffix(reply: &str, sentence_index: usize) -> Option<String> {
    if reply.len() > 16_000 || sentence_index >= 80 {
        return None;
    }
    let normalized = reply.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut sentences = Vec::new();
    let mut start = 0;
    let mut iter = normalized.char_indices().peekable();
    while let Some((index, character)) = iter.next() {
        if !matches!(character, '.' | '!' | '?' | '…') {
            continue;
        }
        let mut end = index + character.len_utf8();
        while let Some(&(next_index, next_char)) = iter.peek() {
            if matches!(next_char, '.' | '!' | '?' | '…') {
                end = next_index + next_char.len_utf8();
                iter.next();
            } else {
                break;
            }
        }
        if normalized[end..].is_empty()
            || normalized[end..].chars().next().is_some_and(char::is_whitespace)
        {
            let sentence = normalized[start..end].trim();
            if !sentence.is_empty() {
                sentences.push(sentence);
            }
            start = end;
        }
    }
    let remaining = normalized[start..].trim();
    if !remaining.is_empty() {
        sentences.push(remaining);
    }
    if sentences.is_empty() || sentences.len() > 80 || sentence_index >= sentences.len() {
        return None;
    }
    let suffix = sentences[sentence_index..].join(" ");
    (!suffix.is_empty() && suffix.len() <= 16_000).then_some(suffix)
}

pub(super) fn route_post(
    path: &str,
    request: &mut Request,
    state: &AppState,
) -> Option<Result<HttpResponse, HttpResponse>> {
    match path {
        "/api/avatar/resume-answer" => Some(
            parse_json::<ResumeAnswerBody>(request).and_then(|body| {
                reject_if_session_ending(state)?;
                if state.voice_busy.load(Ordering::Acquire) {
                    return Err(super::error_response(409, "INVALID_STATE_TRANSITION"));
                }
                let mut source = state.replay_source.lock();
                let reply = source.as_mut().ok_or_else(|| {
                    super::error_response(409, "INVALID_STATE_TRANSITION")
                })?;
                if reply.request_sequence != body.request_sequence
                    || reply.resume_count >= MAX_RESUMES_PER_REPLY
                {
                    return Err(super::error_response(409, "INVALID_STATE_TRANSITION"));
                }
                let suffix = authorized_sentence_suffix(&reply.reply, body.sentence_index)
                    .ok_or_else(|| super::error_response(400, "INVALID_INPUT"))?;
                let result = with_engine_result(state, |engine| {
                    if engine.status().session_state != "active" {
                        return Err(vpr_owner_lab::LabError::InvalidState);
                    }
                    engine
                        .prepare_resumed_speech(&suffix)
                        .map(|response| super::json_response(200, &response))
                })?;
                reply.resume_count += 1;
                Ok(result)
            }),
        ),
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
                        .map(|command| {
                            if let Some(handle) = state.active_voice_interrupt.lock().clone() {
                                let _ = handle.interrupt();
                            }
                            super::json_response(200, &command)
                        })
                })
            }))
        }
        "/api/avatar/client-delivery-sent" => Some(
            parse_json::<ClientDeliverySentBody>(request).and_then(|body| {
                reject_if_session_ending(state)?;
                state
                    .voice_playback
                    .acknowledge_voice_delivery_sent(
                        body.evidence_turn_sequence,
                        body.evidence_output_sequence,
                    )
                    .map(|()| super::json_response(200, &serde_json::json!({"ok": true})))
                    .map_err(|error| super::lab_error_response(&error))
            }),
        ),
        _ => None,
    }
}

// No real provider calls: suffix selection must match the browser's bounded
// in-memory sentence selector and must fail for wrong/out-of-range indices.
#[cfg(test)]
mod resume_tests {
    use super::authorized_sentence_suffix;

    #[test]
    fn choose_only_authoritative_remaining_sentences() {
        let reply = "Сначала уточню один важный момент, затем продолжу. Третья фраза.";
        assert_eq!(
            authorized_sentence_suffix(reply, 1).as_deref(),
            Some("Третья фраза.")
        );
        assert_eq!(authorized_sentence_suffix(reply, 2), None);
    }

    #[test]
    fn cannot_replay_oversized_or_missing_sentence() {
        assert!(authorized_sentence_suffix(&"x".repeat(16_001), 0).is_none());
        assert!(authorized_sentence_suffix("Привет.", 80).is_none());
        assert!(authorized_sentence_suffix("  ", 0).is_none());
        assert_eq!(
            authorized_sentence_suffix("Первое! Второе? Третье...", 2).as_deref(),
            Some("Третье...")
        );
    }
}
