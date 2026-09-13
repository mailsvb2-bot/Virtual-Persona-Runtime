use vpr_domain::{OutputDeliveryState, OutputEvidence, Rt0ReasonCode, Turn, TurnState};

use crate::media_timeline::{MediaTimeline, MediaTimelineError, TurnMediaState};
use crate::output::{OutputSegmentEvidence, OutputSegmentId};
use vpr_integration::MediaTimelineStamp;

#[derive(Debug)]
pub(crate) struct TurnMutableState {
    turn: Turn,
    output_segments: Vec<OutputSegmentEvidence>,
    next_segment_id: u64,
    media: TurnMediaState,
}

impl TurnMutableState {
    #[must_use]
    pub(crate) fn new(turn: Turn, media_epoch: u64) -> Self {
        Self {
            turn,
            output_segments: Vec::new(),
            next_segment_id: 1,
            media: TurnMediaState::new(media_epoch),
        }
    }

    #[must_use]
    pub(crate) fn state(&self) -> TurnState {
        self.turn.state()
    }

    #[must_use]
    pub(crate) fn output_segments(&self) -> Vec<OutputSegmentEvidence> {
        self.output_segments.clone()
    }

    #[must_use]
    pub(crate) fn provider_execution_allowed(&self) -> bool {
        matches!(
            self.turn.state(),
            TurnState::Authorized | TurnState::Processing | TurnState::Outputting
        )
    }

    pub(crate) fn transition(&mut self, next: TurnState) -> Result<(), Rt0ReasonCode> {
        self.turn
            .transition(next)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    pub(crate) fn transition_and_freeze(&mut self, next: TurnState) -> Result<(), Rt0ReasonCode> {
        self.transition(next)?;
        self.freeze_output_segments()
    }

    pub(crate) fn begin_output_segment(&mut self) -> Result<OutputSegmentId, Rt0ReasonCode> {
        if self.turn.state() != TurnState::Outputting {
            return Err(Rt0ReasonCode::InvalidStateTransition);
        }
        let id = OutputSegmentId(self.next_segment_id);
        self.next_segment_id = self
            .next_segment_id
            .checked_add(1)
            .ok_or(Rt0ReasonCode::InternalError)?;
        let mut evidence = OutputEvidence::default();
        evidence
            .mark_generated()
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)?;
        self.output_segments
            .push(OutputSegmentEvidence { id, evidence });
        Ok(id)
    }

    #[cfg(test)]
    pub(crate) fn mark_output_sent(&mut self, id: OutputSegmentId) -> Result<(), Rt0ReasonCode> {
        self.require_outputting()?;
        self.segment_mut(id)?
            .evidence
            .mark_sent()
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    #[cfg(test)]
    pub(crate) fn mark_output_played(&mut self, id: OutputSegmentId) -> Result<(), Rt0ReasonCode> {
        self.require_outputting()?;
        self.segment_mut(id)?
            .evidence
            .mark_played()
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    pub(crate) fn reconcile_output_delivery_uncertain(
        &mut self,
        id: OutputSegmentId,
    ) -> Result<(), Rt0ReasonCode> {
        self.segment_mut(id)?
            .evidence
            .mark_delivery_uncertain()
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    pub(crate) fn reconcile_output_sent(
        &mut self,
        id: OutputSegmentId,
    ) -> Result<(), Rt0ReasonCode> {
        self.segment_mut(id)?
            .evidence
            .reconcile_sent()
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    pub(crate) fn reconcile_output_played(
        &mut self,
        id: OutputSegmentId,
    ) -> Result<(), Rt0ReasonCode> {
        self.segment_mut(id)?
            .evidence
            .reconcile_played()
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    pub(crate) fn next_audio_media_stamp(
        &mut self,
        timeline: &MediaTimeline,
        now_millis: u64,
        duration_micros: u64,
    ) -> Result<MediaTimelineStamp, Rt0ReasonCode> {
        if self.turn.state() != TurnState::Outputting {
            return Err(Rt0ReasonCode::InvalidStateTransition);
        }
        self.media
            .next_audio_stamp(timeline, now_millis, duration_micros)
            .map_err(map_media_timeline_error)
    }

    pub(crate) fn next_video_media_stamp(
        &mut self,
        timeline: &MediaTimeline,
        now_millis: u64,
        source_timestamp_micros: u64,
    ) -> Result<MediaTimelineStamp, Rt0ReasonCode> {
        if self.turn.state() != TurnState::Outputting {
            return Err(Rt0ReasonCode::InvalidStateTransition);
        }
        self.media
            .next_video_stamp(timeline, now_millis, source_timestamp_micros)
            .map_err(map_media_timeline_error)
    }

    #[must_use]
    pub(crate) fn media_flush_context(&self) -> (u64, Option<u64>) {
        self.media.flush_context()
    }

    pub(crate) fn interrupt(&mut self) -> Result<(), Rt0ReasonCode> {
        let interruptible = matches!(
            self.turn.state(),
            TurnState::Received
                | TurnState::Authorized
                | TurnState::Processing
                | TurnState::Outputting
        );
        let already_cancelled = self.output_segments.iter().any(|segment| {
            matches!(
                segment.evidence.state(),
                OutputDeliveryState::Cancelled { .. }
                    | OutputDeliveryState::CancelledDeliveryUncertain
            )
        });
        if !interruptible || already_cancelled {
            return Err(Rt0ReasonCode::InvalidStateTransition);
        }
        self.transition(TurnState::Cancelled)?;
        for segment in &mut self.output_segments {
            segment
                .evidence
                .mark_cancelled()
                .map_err(|_| Rt0ReasonCode::InvalidStateTransition)?;
        }
        Ok(())
    }

    fn freeze_output_segments(&mut self) -> Result<(), Rt0ReasonCode> {
        for segment in &mut self.output_segments {
            if !matches!(
                segment.evidence.state(),
                OutputDeliveryState::Cancelled { .. }
                    | OutputDeliveryState::CancelledDeliveryUncertain
            ) {
                segment
                    .evidence
                    .mark_cancelled()
                    .map_err(|_| Rt0ReasonCode::InvalidStateTransition)?;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn require_outputting(&self) -> Result<(), Rt0ReasonCode> {
        if self.turn.state() == TurnState::Outputting {
            Ok(())
        } else {
            Err(Rt0ReasonCode::InvalidStateTransition)
        }
    }

    fn segment_mut(
        &mut self,
        id: OutputSegmentId,
    ) -> Result<&mut OutputSegmentEvidence, Rt0ReasonCode> {
        self.output_segments
            .iter_mut()
            .find(|segment| segment.id == id)
            .ok_or(Rt0ReasonCode::InvalidStateTransition)
    }
}

fn map_media_timeline_error(error: MediaTimelineError) -> Rt0ReasonCode {
    match error {
        MediaTimelineError::StaleEpoch | MediaTimelineError::TimestampRegression => {
            Rt0ReasonCode::InvalidStateTransition
        }
        MediaTimelineError::Overflow => Rt0ReasonCode::InternalError,
    }
}
