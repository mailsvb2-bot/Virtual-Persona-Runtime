use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use parking_lot::{Mutex as ParkingMutex, MutexGuard as ParkingMutexGuard};

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
    /// Only a client-confirmed stop may make this reply eligible for replay.
    pub interrupted: bool,
}

/// Publish a completed canonical reply only while its session is still live.
/// Session teardown sets the atomic termination intent before clearing this
/// cache, so a late STT/LLM completion must not resurrect replay authority.
// The existing text-completion path uses this mutex-owning helper;
// voice completion uses the locked variant to fence its session epoch.
pub(super) fn retain_completed_reply(
    source: &ParkingMutex<Option<AuthorizedReply>>,
    session_end_requested: &AtomicBool,
    request_sequence: u64,
    reply: &str,
) {
    let mut cached = source.lock();
    retain_completed_reply_locked(&mut cached, session_end_requested, request_sequence, reply);
}

/// Acquire the existing replay lock and validate the originating session under
/// that SAME lock used by session registration and revoke. A stale completion
/// may not remove or overwrite another session's saved answer.
pub(super) fn begin_voice_terminal_replay<'a>(
    source: &'a ParkingMutex<Option<AuthorizedReply>>,
    active_session_sequence: &AtomicU64,
    session_end_requested: &AtomicBool,
    expected_session_sequence: u64,
) -> Option<ParkingMutexGuard<'a, Option<AuthorizedReply>>> {
    let cached = source.lock();
    if expected_session_sequence == 0
        || active_session_sequence.load(Ordering::Acquire) != expected_session_sequence
        || session_end_requested.load(Ordering::Acquire)
    {
        return None;
    }
    Some(cached)
}

/// Called only with the canonical replay_source lock held. The voice worker
/// verifies its session generation while holding this same lock, so an old
/// result cannot overwrite a newer session's replay entitlement.
pub(super) fn retain_completed_reply_locked(
    cached: &mut Option<AuthorizedReply>,
    session_end_requested: &AtomicBool,
    request_sequence: u64,
    reply: &str,
) {
    if session_end_requested.load(Ordering::Acquire)
        || reply.trim().is_empty()
        || reply.len() > 16_000
    {
        *cached = None;
        return;
    }
    *cached = Some(AuthorizedReply {
        request_sequence,
        reply: reply.to_owned(),
        resume_count: 0,
        interrupted: false,
    });
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GreetingBody {
    expected_session_sequence: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResumeAnswerBody {
    request_sequence: u64,
    sentence_index: usize,
    #[serde(default)]
    character_offset: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmInterruptionBody {
    request_sequence: u64,
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
            || normalized[end..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
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

/// The cursor is a Unicode scalar offset into the SERVER-held normalized reply.
/// It can only start at a word boundary and cannot introduce arbitrary browser text.
fn authorized_character_suffix(reply: &str, offset: usize) -> Option<String> {
    if reply.len() > 16_000 {
        return None;
    }
    let normalized = reply.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars = normalized.chars().collect::<Vec<_>>();
    if offset >= chars.len() || offset > 16_000 {
        return None;
    }
    // A resume cursor may begin a word after whitespace or after standalone
    // punctuation, but never inside a date, decimal, or attached word.
    if offset > 0 && !chars[offset - 1].is_whitespace() {
        let previous = chars[offset - 1];
        let punctuation_boundary = matches!(
            previous,
            '.' | '!' | '?' | '…' | ',' | ':' | ';' | '—' | '–' | '(' | ')' | '«' | '»'
        ) && !chars[offset].is_alphanumeric();
        if !punctuation_boundary {
            return None;
        }
    }
    let suffix = chars[offset..].iter().collect::<String>();
    let suffix = suffix.trim();
    (!suffix.is_empty() && suffix.len() <= 16_000).then(|| suffix.to_owned())
}

fn confirm_interruption(
    request: &mut Request,
    state: &AppState,
) -> Result<HttpResponse, HttpResponse> {
    let body = parse_json::<ConfirmInterruptionBody>(request)?;
    reject_if_session_ending(state)?;
    if state.voice_busy.load(Ordering::Acquire) {
        return Err(super::error_response(409, "INVALID_STATE_TRANSITION"));
    }
    let mut source = state.replay_source.lock();
    let reply = source
        .as_mut()
        .ok_or_else(|| super::error_response(409, "INVALID_STATE_TRANSITION"))?;
    if reply.request_sequence != body.request_sequence {
        return Err(super::error_response(409, "INVALID_STATE_TRANSITION"));
    }
    with_engine_result(state, |engine| {
        if engine.status().session_state != "active" {
            return Err(vpr_owner_lab::LabError::InvalidState);
        }
        Ok(super::json_response(200, &serde_json::json!({"ok": true})))
    })?;
    reject_if_session_ending(state)?;
    reply.interrupted = true;
    Ok(super::json_response(200, &serde_json::json!({"ok": true})))
}

fn greeting_response(
    request: &mut Request,
    state: &AppState,
) -> Result<HttpResponse, HttpResponse> {
    let body = parse_json::<GreetingBody>(request)?;
    reject_if_session_ending(state)?;
    let current = state.active_session_sequence.load(Ordering::Acquire);
    if current == 0 || current != body.expected_session_sequence {
        return Err(super::error_response(409, "INVALID_STATE_TRANSITION"));
    }
    if state
        .voice_busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(super::error_response(409, "INVALID_STATE_TRANSITION"));
    }
    let _busy =
        super::http_evidence::VoiceBusyGuard::new(&state.voice_busy, &state.voice_cancel_requested);
    reject_if_session_ending(state)?;
    if state
        .greeting_session_sequence
        .compare_exchange(0, current, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(super::error_response(409, "GREETING_ALREADY_SENT"));
    }
    let result = with_engine_result(state, |engine| {
        if engine.status().session_state != "active" {
            return Err(vpr_owner_lab::LabError::InvalidState);
        }
        engine
            .prepare_connection_greeting()
            .map(|response| super::json_response(200, &response))
    });
    if result.is_err() {
        let _ = state.greeting_session_sequence.compare_exchange(
            current,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
    result
}

pub(super) fn route_post(
    path: &str,
    request: &mut Request,
    state: &AppState,
) -> Option<Result<HttpResponse, HttpResponse>> {
    match path {
        "/api/avatar/greeting" => Some(greeting_response(request, state)),
        "/api/avatar/confirm-interruption" => Some(confirm_interruption(request, state)),
        "/api/avatar/resume-answer" => {
            Some(parse_json::<ResumeAnswerBody>(request).and_then(|body| {
                reject_if_session_ending(state)?;
                if state
                    .voice_busy
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    return Err(super::error_response(409, "INVALID_STATE_TRANSITION"));
                }
                let _busy = super::http_evidence::VoiceBusyGuard::new(
                    &state.voice_busy,
                    &state.voice_cancel_requested,
                );
                reject_if_session_ending(state)?;
                let mut source = state.replay_source.lock();
                let reply = source
                    .as_mut()
                    .ok_or_else(|| super::error_response(409, "INVALID_STATE_TRANSITION"))?;
                if reply.request_sequence != body.request_sequence
                    || !reply.interrupted
                    || reply.resume_count >= MAX_RESUMES_PER_REPLY
                {
                    return Err(super::error_response(409, "INVALID_STATE_TRANSITION"));
                }
                let suffix = match body.character_offset {
                    Some(offset) => authorized_character_suffix(&reply.reply, offset),
                    None => authorized_sentence_suffix(&reply.reply, body.sentence_index),
                }
                .ok_or_else(|| super::error_response(400, "INVALID_INPUT"))?;
                let result = with_engine_result(state, |engine| {
                    if engine.status().session_state != "active" {
                        return Err(vpr_owner_lab::LabError::InvalidState);
                    }
                    engine
                        .prepare_resumed_speech(&suffix)
                        .map(|response| super::json_response(200, &response))
                })?;
                // End-session writes the atomic intent BEFORE acquiring replay_source.
                // Reject a revoke that began during the potentially slow provider
                // preparation, rather than handing its command to the browser.
                reject_if_session_ending(state)?;
                reply.resume_count += 1;
                reply.interrupted = false;
                Ok(result)
            }))
        }
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
    use super::{authorized_character_suffix, authorized_sentence_suffix};

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

    #[test]
    fn late_stream_completion_cannot_restore_reply_after_revoke() {
        use parking_lot::Mutex;
        use std::sync::atomic::{AtomicBool, Ordering};

        let source = Mutex::new(None);
        let ending = AtomicBool::new(false);
        super::retain_completed_reply(&source, &ending, 1, "Первый ответ.");
        assert_eq!(
            source.lock().as_ref().map(|item| item.request_sequence),
            Some(1)
        );
        assert_eq!(
            source.lock().as_ref().map(|item| item.interrupted),
            Some(false)
        );

        ending.store(true, Ordering::Release);
        *source.lock() = None;
        super::retain_completed_reply(&source, &ending, 1, "Поздний ответ.");
        assert!(
            source.lock().is_none(),
            "late completion must not revive revoked replay"
        );
    }

    #[test]
    fn late_previous_session_terminal_cannot_clear_or_reauthorize_new_reply() {
        use parking_lot::Mutex;
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

        let source = Mutex::new(None);
        let active = AtomicU64::new(1);
        let ending = AtomicBool::new(false);

        {
            let mut replay =
                super::begin_voice_terminal_replay(&source, &active, &ending, 1).unwrap();
            super::retain_completed_reply_locked(&mut replay, &ending, 7, "Старый ответ.");
        }
        // A new session replaced the cached answer under the same canonical
        // mutex. The old worker's completion is not allowed to clear it.
        {
            let mut replay = source.lock();
            active.store(2, Ordering::Release);
            super::retain_completed_reply_locked(&mut replay, &ending, 8, "Ответ новой сессии.");
        }
        assert!(super::begin_voice_terminal_replay(&source, &active, &ending, 1).is_none());
        assert_eq!(source.lock().as_ref().unwrap().request_sequence, 8);
        assert_eq!(source.lock().as_ref().unwrap().reply, "Ответ новой сессии.");

        ending.store(true, Ordering::Release);
        assert!(super::begin_voice_terminal_replay(&source, &active, &ending, 2).is_none());
        assert!(super::begin_voice_terminal_replay(&source, &active, &ending, 0).is_none());
    }

    #[test]
    fn replay_cursor_never_splits_dates_decimals_or_attached_words() {
        let reply = "Встреча 12.10.2026 и оценка 3.14. Следующий пункт.";
        for prefix in [
            "Встреча 12.",
            "Встреча 12.10.",
            "Встреча 12.10.2026 и оценка 3.",
        ] {
            assert!(
                authorized_character_suffix(reply, prefix.chars().count()).is_none(),
                "cursor must not split a numeric token"
            );
        }
        let safe_offset = "Встреча 12.10.2026 и оценка 3.14. ".chars().count();
        assert_eq!(
            authorized_character_suffix(reply, safe_offset).as_deref(),
            Some("Следующий пункт.")
        );
    }

    #[test]
    fn unicode_word_cursor_replays_only_authorized_suffix() {
        let reply = "Первая мысль. Дальше важная мысль. Третья мысль.";
        let offset = "Первая мысль. Дальше ".chars().count();
        assert_eq!(
            authorized_character_suffix(reply, offset).as_deref(),
            Some("важная мысль. Третья мысль.")
        );
        assert_eq!(
            authorized_character_suffix(reply, offset + 2),
            None,
            "mid-word positions rejected"
        );
        assert_eq!(
            authorized_character_suffix(reply, reply.chars().count()),
            None
        );
        assert_eq!(authorized_character_suffix(reply, usize::MAX), None);
        assert_eq!(authorized_character_suffix("  ", 0), None);
    }

    #[test]
    fn no_replay_source_for_empty_or_oversized_reply() {
        use parking_lot::Mutex;
        use std::sync::atomic::AtomicBool;

        let source = Mutex::new(None);
        let ending = AtomicBool::new(false);
        super::retain_completed_reply(&source, &ending, 10, "Текущий ответ.");
        super::retain_completed_reply(&source, &ending, 10, "   ");
        assert!(source.lock().is_none());
        super::retain_completed_reply(&source, &ending, 10, &"x".repeat(16_001));
        assert!(source.lock().is_none());
    }
}
