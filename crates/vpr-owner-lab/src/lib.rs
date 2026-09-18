mod evidence;
mod owner_capture;
mod owner_context;
mod providers;
mod state;

pub use evidence::{
    LabAvSyncEvidenceInput, LabAvSyncReference, LabEvidenceError, LabMediaEvidenceInput,
    LabMediaEvidenceKind, LabSessionEvidenceRecorder, LabSessionEvidenceSnapshot,
    LabTextAttemptEvidence, LabTextAttemptStatus, LabVoiceAttemptEvidence, LabVoiceAttemptStatus,
    ParticipantRole, RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA,
};

pub use owner_capture::{
    OwnerCaptureClaim, OwnerCaptureError, OwnerCaptureQuestion, OwnerCaptureSnapshot,
    Rt0OwnerCapture,
};

pub use owner_context::{ReviewedOwnerClaimSnapshot, ReviewedOwnerContextSnapshot};

pub use state::{
    ConversationReadiness, LabClientCommand, LabClientControl, LabError, LabProviderUsage,
    LabSessionAudience, LabSignalBundle, LabStatus, LabTextResult, LabVoiceResult, LabVoiceUsage,
    OwnerContextState, OwnerLabEngine, OwnerLabStartRequest, OwnerLabTurnInput,
};

pub use providers::{
    PreparedTtsProbe, ProviderBundle, ProviderDescriptor, build_tts_probe_from_env,
};
