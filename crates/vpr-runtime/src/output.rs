use vpr_domain::{OutputDeliveryState, OutputEvidence};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OutputSegmentId(pub(crate) u64);

impl OutputSegmentId {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputSegmentEvidence {
    pub(crate) id: OutputSegmentId,
    pub(crate) evidence: OutputEvidence,
}

impl OutputSegmentEvidence {
    #[must_use]
    pub const fn id(self) -> OutputSegmentId {
        self.id
    }

    #[must_use]
    pub fn state(self) -> OutputDeliveryState {
        self.evidence.state()
    }

    #[must_use]
    pub fn eligible_as_spoken(self) -> bool {
        self.evidence.eligible_as_spoken()
    }
}
