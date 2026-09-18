mod binding;
mod exit;
mod exit_assembly;
mod exit_context;
mod exit_support;
mod exit_validation;
mod golden;
mod known_limitations;
mod live_provider;
mod session_binding;
mod session_evidence;
mod supporting_preflight;

pub use binding::{
    BoundGoldenReport, EvidenceBinding, EvidenceBindingError, EvidenceVerificationContext,
    GoldenEvidenceBundle, ProviderRole, ProviderStateBinding, ProviderStateManifest,
    RT0_EVIDENCE_BINDING_SCHEMA, RT0_PROVIDER_STATE_SCHEMA, evaluate_bound_golden_suite,
    sha256_hex,
};

pub use golden::{
    AttributionEvidence, DerivationEvidence, GoldenActor, GoldenCase, GoldenCaseResult,
    GoldenExpectation, GoldenFailureCode, GoldenObservation, GoldenReport, GoldenSuite,
    GoldenSuiteError, RT0_GOLDEN_SCHEMA, SourceEvidence, VerificationEvidence,
    evaluate_golden_suite,
};

pub use live_provider::{
    AvatarProbeEvidence, LiveProviderProbeReceipt, LlmProbeEvidence, ProbeUsage,
    RT0_LIVE_PROVIDER_PROBE_SCHEMA, SttProbeEvidence, TtsProbeEvidence,
};

pub use exit::{
    AcceptanceEvidence, ArtifactCheckEvidence, AutomatedEvidence, CheckStatus,
    ConversationEvidence, ConversationPairEvidence, CostEvidence, EvidenceOrigin, HumanDimensions,
    HumanEvaluationEvidence, KnownLimitationsEvidence, LatencyDistributionMillis, ParticipantRole,
    PrivacyPermissionEvidence, QualityEvidence, RT0_EXIT_EVIDENCE_SCHEMA, RT0_EXIT_REPORT_SCHEMA,
    RecordStatus, Rt0ExitEvidence, Rt0ExitEvidenceError, Rt0ExitFailureCode, Rt0ExitReport,
    evaluate_rt0_exit_evidence,
};
pub use exit_assembly::{Rt0ExitAssemblyError, Rt0ExitAssemblyInputs, assemble_rt0_exit_evidence};
pub use exit_context::Rt0ExitVerificationContext;
pub use exit_support::{
    Rt0ExitSupportingArtifacts, evaluate_verified_rt0_exit_evidence,
    validate_rt0_exit_supporting_artifacts,
};
pub use exit_validation::validate_rt0_conversation_evidence_binding;
pub use session_binding::{
    BoundLabSessionEvidenceAggregate, LabSessionBindingError, RT0_OWNER_LAB_SESSION_BINDING_SCHEMA,
    bind_owner_lab_session_evidence,
};
pub use session_evidence::{
    LabAvSyncEvidence, LabAvSyncEvidenceInput, LabAvSyncReference, LabMediaEvidence,
    LabMediaEvidenceInput, LabMediaEvidenceKind, LabSessionAggregateError,
    LabSessionEvidenceAggregate, LabSessionEvidenceSnapshot, LabTextAttemptEvidence,
    LabTextAttemptStatus, LabVoiceAttemptEvidence, LabVoiceAttemptStatus,
    RT0_AV_SYNC_SAMPLES_PER_REQUEST, RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE,
    RT0_OWNER_LAB_SESSION_AGGREGATE_SCHEMA, RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA,
    SessionUsageEvidence, aggregate_owner_lab_session_evidence,
};

pub use supporting_preflight::{
    RT0_SUPPORTING_PREFLIGHT_SCHEMA, Rt0SupportingArtifactDigests, Rt0SupportingPreflightArtifacts,
    Rt0SupportingPreflightError, Rt0SupportingPreflightReport, preflight_rt0_supporting_artifacts,
};
