use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use serde::Serialize;
use vpr_owner_lab::LabError;

use super::voice_event::VoiceStreamEvent;

const EVENT_WAIT_TIMEOUT: Duration = Duration::from_secs(25);
const TERMINATION_WAIT_TIMEOUT: Duration = Duration::from_secs(1);
// Keep a just-completed reply pollable while the next voice turn runs.
// If two terminal replies are unconsumed, reject new turns instead of
// silently deleting the evidence required by their clients.
const MAX_RETAINED_VOICE_STREAMS: usize = 2;
pub(super) const MAX_PENDING_VOICE_STREAM_EVENTS: usize = 64;

#[derive(Default)]
struct VoiceStreamState {
    events: VecDeque<VoiceStreamEvent>,
    terminal: bool,
}

#[derive(Default)]
pub(crate) struct VoiceStreamRegistry {
    streams: Mutex<BTreeMap<(u64, u64), VoiceStreamState>>,
    changed: Condvar,
}

impl VoiceStreamRegistry {
    pub(crate) fn clear(&self) {
        self.streams.lock().clear();
        self.changed.notify_all();
    }

    pub(crate) fn wait_until_quiescent(&self) -> bool {
        let started = std::time::Instant::now();
        let mut streams = self.streams.lock();
        loop {
            if streams.values().all(|stream| stream.terminal) {
                return true;
            }
            let Some(remaining) = TERMINATION_WAIT_TIMEOUT.checked_sub(started.elapsed()) else {
                return false;
            };
            if remaining.is_zero() {
                return false;
            }
            if self.changed.wait_for(&mut streams, remaining).timed_out()
                && streams.values().any(|stream| !stream.terminal)
            {
                return false;
            }
        }
    }

    pub(crate) fn begin(&self, session_sequence: u64, request_sequence: u64) -> bool {
        if session_sequence == 0 || request_sequence == 0 {
            return false;
        }
        let mut streams = self.streams.lock();
        if streams.len() >= MAX_RETAINED_VOICE_STREAMS
            || streams.contains_key(&(session_sequence, request_sequence))
            || streams.values().any(|stream| !stream.terminal)
        {
            return false;
        }
        streams.insert((session_sequence, request_sequence), VoiceStreamState::default());
        true
    }

    pub(super) fn push(
        &self,
        session_sequence: u64,
        request_sequence: u64,
        event: VoiceStreamEvent,
    ) -> Result<(), LabError> {
        let mut streams = self.streams.lock();
        let stream = streams
            .get_mut(&(session_sequence, request_sequence))
            .ok_or(LabError::InvalidState)?;
        if stream.terminal
            || stream.events.len() >= MAX_PENDING_VOICE_STREAM_EVENTS.saturating_sub(1)
        {
            return Err(LabError::InvalidState);
        }
        stream.events.push_back(event);
        drop(streams);
        self.changed.notify_all();
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn finish(&self, session_sequence: u64, request_sequence: u64, event: VoiceStreamEvent) {
        self.finish_with_unlock(session_sequence, request_sequence, event, || {});
    }

    /// Publish the terminal event and release the old voice turn as one
    /// state transition: a new turn cannot observe busy=false while the
    /// old stream still appears non-terminal in this registry.
    pub(super) fn finish_with_unlock(
        &self,
        session_sequence: u64,
        request_sequence: u64,
        event: VoiceStreamEvent,
        release_turn: impl FnOnce(),
    ) {
        let mut streams = self.streams.lock();
        if let Some(stream) = streams.get_mut(&(session_sequence, request_sequence)) {
            debug_assert!(stream.events.len() < MAX_PENDING_VOICE_STREAM_EVENTS);
            stream.events.push_back(event);
            stream.terminal = true;
        }
        release_turn();
        drop(streams);
        self.changed.notify_all();
    }

    pub(super) fn wait_events(&self, session_sequence: u64, request_sequence: u64) -> Option<VoiceEventsResponse> {
        let mut streams = self.streams.lock();
        {
            let stream = streams.get(&(session_sequence, request_sequence))?;
            if stream.events.is_empty() && !stream.terminal {
                self.changed.wait_for(&mut streams, EVENT_WAIT_TIMEOUT);
            }
        }
        let (events, terminal) = {
            let stream = streams.get_mut(&(session_sequence, request_sequence))?;
            (stream.events.drain(..).collect::<Vec<_>>(), stream.terminal)
        };
        if terminal {
            streams.remove(&(session_sequence, request_sequence));
        }
        Some(VoiceEventsResponse { events, terminal })
    }
}

#[derive(Serialize)]
pub(super) struct VoiceEventsResponse {
    pub(super) events: Vec<VoiceStreamEvent>,
    pub(super) terminal: bool,
}
