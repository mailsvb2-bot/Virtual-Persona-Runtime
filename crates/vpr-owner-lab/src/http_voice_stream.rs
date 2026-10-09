use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use serde::Serialize;
use vpr_owner_lab::LabError;

use super::voice_event::VoiceStreamEvent;

const EVENT_WAIT_TIMEOUT: Duration = Duration::from_secs(25);
const TERMINATION_WAIT_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_RETAINED_VOICE_STREAMS: usize = 1;
pub(super) const MAX_PENDING_VOICE_STREAM_EVENTS: usize = 64;

#[derive(Default)]
struct VoiceStreamState {
    events: VecDeque<VoiceStreamEvent>,
    terminal: bool,
}

#[derive(Default)]
pub(super) struct VoiceStreamRegistry {
    streams: Mutex<BTreeMap<u64, VoiceStreamState>>,
    changed: Condvar,
}

impl VoiceStreamRegistry {
    pub(super) fn clear(&self) {
        self.streams.lock().clear();
        self.changed.notify_all();
    }

    pub(super) fn wait_until_quiescent(&self) -> bool {
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

    pub(super) fn begin(&self, request_sequence: u64) -> bool {
        let mut streams = self.streams.lock();
        streams.retain(|_, stream| !stream.terminal);
        if streams.len() >= MAX_RETAINED_VOICE_STREAMS || streams.contains_key(&request_sequence) {
            return false;
        }
        streams.insert(request_sequence, VoiceStreamState::default());
        true
    }

    pub(super) fn push(&self, request_sequence: u64, event: VoiceStreamEvent) -> Result<(), LabError> {
        let mut streams = self.streams.lock();
        let stream = streams
            .get_mut(&request_sequence)
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
    pub(super) fn finish(&self, request_sequence: u64, event: VoiceStreamEvent) {
        self.finish_with_unlock(request_sequence, event, || {});
    }

    /// Publish the terminal event and release the old voice turn as one
    /// state transition: a new turn cannot observe busy=false while the
    /// old stream still appears non-terminal in this registry.
    pub(super) fn finish_with_unlock(
        &self,
        request_sequence: u64,
        event: VoiceStreamEvent,
        release_turn: impl FnOnce(),
    ) {
        let mut streams = self.streams.lock();
        if let Some(stream) = streams.get_mut(&request_sequence) {
            debug_assert!(stream.events.len() < MAX_PENDING_VOICE_STREAM_EVENTS);
            stream.events.push_back(event);
            stream.terminal = true;
        }
        release_turn();
        drop(streams);
        self.changed.notify_all();
    }

    pub(super) fn wait_events(&self, request_sequence: u64) -> Option<VoiceEventsResponse> {
        let mut streams = self.streams.lock();
        {
            let stream = streams.get(&request_sequence)?;
            if stream.events.is_empty() && !stream.terminal {
                self.changed.wait_for(&mut streams, EVENT_WAIT_TIMEOUT);
            }
        }
        let (events, terminal) = {
            let stream = streams.get_mut(&request_sequence)?;
            (stream.events.drain(..).collect::<Vec<_>>(), stream.terminal)
        };
        if terminal {
            streams.remove(&request_sequence);
        }
        Some(VoiceEventsResponse { events, terminal })
    }
}

#[derive(Serialize)]
pub(super) struct VoiceEventsResponse {
    pub(super) events: Vec<VoiceStreamEvent>,
    pub(super) terminal: bool,
}
