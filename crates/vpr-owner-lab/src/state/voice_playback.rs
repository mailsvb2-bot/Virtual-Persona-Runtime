use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::Mutex;
use vpr_runtime::{ActiveTurn, OutputDeliveryHandle};

use super::LabError;

struct PendingVoicePlayback {
    turn: Arc<ActiveTurn>,
    deliveries: BTreeMap<u64, OutputDeliveryHandle>,
}

#[derive(Clone, Default)]
pub struct LabVoicePlaybackRegistry {
    inner: Arc<Mutex<BTreeMap<u64, PendingVoicePlayback>>>,
}

impl LabVoicePlaybackRegistry {
    pub fn clear(&self) {
        self.inner.lock().clear();
    }

    pub(super) fn register_delivery(
        &self,
        evidence_turn_sequence: u64,
        turn: &Arc<ActiveTurn>,
        delivery: OutputDeliveryHandle,
    ) -> Result<u64, LabError> {
        let sequence = delivery.sequence();
        let mut pending = self.inner.lock();
        // Validate before mutating. BTreeMap::insert replaces the old handle
        // even when reporting a duplicate; that would silently corrupt the
        // canonical sent/played lifecycle after rejecting the registration.
        if let Some(entry) = pending.get_mut(&evidence_turn_sequence) {
            if entry.turn.snapshot().turn_id() != turn.snapshot().turn_id()
                || entry.deliveries.contains_key(&sequence)
            {
                return Err(LabError::InvalidState);
            }
            entry.deliveries.insert(sequence, delivery);
        } else {
            let mut deliveries = BTreeMap::new();
            deliveries.insert(sequence, delivery);
            pending.insert(
                evidence_turn_sequence,
                PendingVoicePlayback {
                    turn: Arc::clone(turn),
                    deliveries,
                },
            );
        }
        Ok(sequence)
    }

    /// Reconciles one browser-confirmed client transport send with a runtime-issued segment.
    ///
    /// # Errors
    /// Returns an invalid-state or runtime lifecycle error for unknown or mismatched segments.
    pub fn acknowledge_voice_delivery_sent(
        &self,
        evidence_turn_sequence: u64,
        evidence_output_sequence: u64,
    ) -> Result<(), LabError> {
        let pending = self.inner.lock();
        let entry = pending
            .get(&evidence_turn_sequence)
            .ok_or(LabError::InvalidState)?;
        let delivery = entry
            .deliveries
            .get(&evidence_output_sequence)
            .ok_or(LabError::InvalidState)?;
        entry
            .turn
            .acknowledge_output_sent(delivery)
            .map_err(LabError::Runtime)
    }

    /// Reconciles one browser-observed audio start with a runtime-issued segment.
    ///
    /// # Errors
    /// Returns an invalid-state or runtime lifecycle error for unknown or mismatched segments.
    pub fn acknowledge_voice_playback(
        &self,
        evidence_turn_sequence: u64,
        evidence_output_sequence: u64,
    ) -> Result<(), LabError> {
        let pending = self.inner.lock();
        let entry = pending
            .get(&evidence_turn_sequence)
            .ok_or(LabError::InvalidState)?;
        let delivery = entry
            .deliveries
            .get(&evidence_output_sequence)
            .ok_or(LabError::InvalidState)?;
        entry
            .turn
            .acknowledge_output_played(delivery)
            .map_err(LabError::Runtime)
    }

    /// Reconciles provider-confirmed completion of the whole browser-controlled playback queue.
    ///
    /// The first output sequence remains the immutable request binding used by sanitized evidence,
    /// while every runtime-issued segment for the same canonical turn is promoted to Played.
    /// This is required for phrase-streamed `LiveKit` turns: a turn is not canonically complete
    /// while any queued segment is merely prepared or sent.
    ///
    /// # Errors
    /// Returns invalid state for an unknown turn/first-output binding or any segment whose runtime
    /// lifecycle cannot be reconciled as played.
    pub fn acknowledge_voice_playback_complete(
        &self,
        evidence_turn_sequence: u64,
        first_output_sequence: u64,
    ) -> Result<(), LabError> {
        let pending = self.inner.lock();
        let entry = pending
            .get(&evidence_turn_sequence)
            .ok_or(LabError::InvalidState)?;
        if !entry.deliveries.contains_key(&first_output_sequence) || entry.deliveries.is_empty() {
            return Err(LabError::InvalidState);
        }
        for delivery in entry.deliveries.values() {
            entry
                .turn
                .acknowledge_output_played(delivery)
                .map_err(LabError::Runtime)?;
        }
        Ok(())
    }
}
