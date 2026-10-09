use serde::Serialize;
use vpr_owner_lab::{LabVoiceResult, LabVoiceSegment};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum VoiceStreamEvent {
    Segment {
        segment: LabVoiceSegment,
    },
    Complete {
        result: Box<LabVoiceResult>,
    },
    Failed {
        code: String,
        diagnostic: Option<String>,
    },
}
