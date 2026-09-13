use std::sync::Arc;

use parking_lot::Mutex;
use vpr_domain::SessionId;
use vpr_integration::MediaTimelineStamp;

#[derive(Debug, Clone)]
pub(crate) struct MediaTimeline {
    session_id: SessionId,
    state: Arc<Mutex<MediaTimelineState>>,
}

#[derive(Debug)]
struct MediaTimelineState {
    epoch: u64,
    next_sequence: u64,
    epoch_floor_micros: u64,
    last_presentation_micros: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MediaTurnOrigin {
    pub(crate) epoch: u64,
    pub(crate) origin_micros: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaTimelineError {
    StaleEpoch,
    TimestampRegression,
    Overflow,
}

#[derive(Debug)]
pub(crate) struct TurnMediaState {
    bound_epoch: u64,
    origin: Option<MediaTurnOrigin>,
    audio_cursor_micros: u64,
    video_source_base_micros: Option<u64>,
    last_video_source_micros: Option<u64>,
    last_sequence: Option<u64>,
}

impl TurnMediaState {
    #[must_use]
    pub(crate) const fn new(bound_epoch: u64) -> Self {
        Self {
            bound_epoch,
            origin: None,
            audio_cursor_micros: 0,
            video_source_base_micros: None,
            last_video_source_micros: None,
            last_sequence: None,
        }
    }

    pub(crate) fn next_audio_stamp(
        &mut self,
        timeline: &MediaTimeline,
        now_millis: u64,
        duration_micros: u64,
    ) -> Result<MediaTimelineStamp, MediaTimelineError> {
        let origin = self.origin(timeline, now_millis)?;
        let relative_micros = self.audio_cursor_micros;
        let stamp = timeline.stamp(origin, relative_micros)?;
        self.audio_cursor_micros = self
            .audio_cursor_micros
            .checked_add(duration_micros)
            .ok_or(MediaTimelineError::Overflow)?;
        self.last_sequence = Some(stamp.sequence);
        Ok(stamp)
    }

    pub(crate) fn next_video_stamp(
        &mut self,
        timeline: &MediaTimeline,
        now_millis: u64,
        source_timestamp_micros: u64,
    ) -> Result<MediaTimelineStamp, MediaTimelineError> {
        if self
            .last_video_source_micros
            .is_some_and(|last| source_timestamp_micros < last)
        {
            return Err(MediaTimelineError::TimestampRegression);
        }
        let origin = self.origin(timeline, now_millis)?;
        let base = self
            .video_source_base_micros
            .unwrap_or(source_timestamp_micros);
        let relative_micros = source_timestamp_micros
            .checked_sub(base)
            .ok_or(MediaTimelineError::TimestampRegression)?;
        let stamp = timeline.stamp(origin, relative_micros)?;
        self.video_source_base_micros
            .get_or_insert(source_timestamp_micros);
        self.last_video_source_micros = Some(source_timestamp_micros);
        self.last_sequence = Some(stamp.sequence);
        Ok(stamp)
    }

    #[must_use]
    pub(crate) fn flush_context(&self) -> (u64, Option<u64>) {
        (
            self.origin.map_or(self.bound_epoch, |origin| origin.epoch),
            self.last_sequence,
        )
    }

    fn origin(
        &mut self,
        timeline: &MediaTimeline,
        now_millis: u64,
    ) -> Result<MediaTurnOrigin, MediaTimelineError> {
        if let Some(origin) = self.origin {
            return Ok(origin);
        }
        let origin = timeline.open_turn_origin(now_millis)?;
        if origin.epoch != self.bound_epoch {
            return Err(MediaTimelineError::StaleEpoch);
        }
        self.origin = Some(origin);
        Ok(origin)
    }
}

impl MediaTimeline {
    #[must_use]
    pub(crate) fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            state: Arc::new(Mutex::new(MediaTimelineState {
                epoch: 1,
                next_sequence: 1,
                epoch_floor_micros: 0,
                last_presentation_micros: None,
            })),
        }
    }

    #[must_use]
    pub(crate) fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub(crate) fn open_turn_origin(
        &self,
        now_millis: u64,
    ) -> Result<MediaTurnOrigin, MediaTimelineError> {
        let now_micros = now_millis
            .checked_mul(1_000)
            .ok_or(MediaTimelineError::Overflow)?;
        let state = self.state.lock();
        let origin_micros = now_micros.max(state.epoch_floor_micros);
        Ok(MediaTurnOrigin {
            epoch: state.epoch,
            origin_micros,
        })
    }

    pub(crate) fn stamp(
        &self,
        origin: MediaTurnOrigin,
        relative_micros: u64,
    ) -> Result<MediaTimelineStamp, MediaTimelineError> {
        let mut state = self.state.lock();
        if origin.epoch != state.epoch {
            return Err(MediaTimelineError::StaleEpoch);
        }
        let presentation_time_micros = origin
            .origin_micros
            .checked_add(relative_micros)
            .ok_or(MediaTimelineError::Overflow)?;
        if state
            .last_presentation_micros
            .is_some_and(|last| presentation_time_micros < last)
        {
            return Err(MediaTimelineError::TimestampRegression);
        }
        let sequence = state.next_sequence;
        state.next_sequence = state
            .next_sequence
            .checked_add(1)
            .ok_or(MediaTimelineError::Overflow)?;
        state.last_presentation_micros = Some(presentation_time_micros);
        Ok(MediaTimelineStamp {
            epoch: state.epoch,
            sequence,
            presentation_time_micros,
        })
    }

    pub(crate) fn rebase(&self, now_millis: u64) -> Result<u64, MediaTimelineError> {
        let now_micros = now_millis
            .checked_mul(1_000)
            .ok_or(MediaTimelineError::Overflow)?;
        let mut state = self.state.lock();
        state.epoch = state
            .epoch
            .checked_add(1)
            .ok_or(MediaTimelineError::Overflow)?;
        let after_last = state
            .last_presentation_micros
            .and_then(|last| last.checked_add(1))
            .unwrap_or(0);
        state.epoch_floor_micros = now_micros.max(after_last);
        Ok(state.epoch)
    }

    #[must_use]
    pub(crate) fn current_epoch(&self) -> u64 {
        self.state.lock().epoch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backwards_clock_fails_closed_until_explicit_rebase() {
        let timeline = MediaTimeline::new(SessionId::new("session-clock").unwrap());
        let first = timeline.open_turn_origin(100).unwrap();
        let first_stamp = timeline.stamp(first, 0).unwrap();
        let backwards = timeline.open_turn_origin(99).unwrap();
        assert_eq!(
            timeline.stamp(backwards, 0),
            Err(MediaTimelineError::TimestampRegression)
        );

        let epoch = timeline.rebase(99).unwrap();
        let rebased = timeline.open_turn_origin(99).unwrap();
        let rebased_stamp = timeline.stamp(rebased, 0).unwrap();
        assert_eq!(rebased_stamp.epoch, epoch);
        assert!(rebased_stamp.sequence > first_stamp.sequence);
        assert!(rebased_stamp.presentation_time_micros > first_stamp.presentation_time_micros);
    }

    #[test]
    fn stale_origin_cannot_emit_after_rebase() {
        let timeline = MediaTimeline::new(SessionId::new("session-stale").unwrap());
        let origin = timeline.open_turn_origin(100).unwrap();
        timeline.rebase(101).unwrap();
        assert_eq!(
            timeline.stamp(origin, 0),
            Err(MediaTimelineError::StaleEpoch)
        );
    }
}
