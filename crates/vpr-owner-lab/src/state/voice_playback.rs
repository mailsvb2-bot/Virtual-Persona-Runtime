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
        turn: Arc<ActiveTurn>,
        delivery: OutputDeliveryHandle,
    ) -> Result<u64, LabError> {
        let sequence = delivery.sequence();
        let mut pending = self.inner.lock();
        let entry = pending
            .entry(evidence_turn_sequence)
            .or_insert_with(|| PendingVoicePlayback {
                turn: Arc::clone(&turn),
                deliveries: BTreeMap::new(),
            });
        if entry.turn.snapshot().turn_id() != turn.snapshot().turn_id()
            || entry.deliveries.insert(sequence, delivery).is_some()
        {
            return Err(LabError::InvalidState);
        }
        Ok(sequence)
    }

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
}
