mod binding;
mod evidence_workspace;
mod exit;
mod exit_assembly;
mod exit_checks;
mod exit_context;
mod exit_cost;
mod exit_support;
mod exit_validation;
mod golden;
mod known_limitations;
mod live_provider;
mod live_readiness;
mod owner_golden;
mod session_binding;
mod session_evidence;
mod session_statistics;
mod session_types;
mod supporting_preflight;
mod supporting_scaffold;

pub use binding::{
    BoundGoldenReport, EvidenceBinding, EvidenceBindingError, EvidenceVerificationContext,
    GoldenEvidenceBundle, ProviderRole, ProviderStateBinding, ProviderStateManifest,
    RT0_EVIDENCE_BINDING_SCHEMA, RT0_PROVIDER_STATE_SCHEMA, evaluate_bound_golden_suite,
    sha256_hex, validate_candidate_sha, validate_provider_state_manifest,
};

pub use evidence_workspace::{
    RT0_EVIDENCE_REQUIRED_FILES, RT0_RELEASE_SPEC_BYTES, Rt0EvidenceWorkspaceError,
    prepare_rt0_evidence_workspace,
};

pub use golden::{
    AttributionEvidence, DerivationEvidence, GoldenActor, GoldenCase, GoldenCaseResult,
    GoldenExpectation, GoldenFailureCode, GoldenObservation, GoldenReport, GoldenSuite,
    GoldenSuiteError, RT0_GOLDEN_SCHEMA, SourceEvidence, VerificationEvidence,
    evaluate_golden_suite,
};

pub use live_provider::{
    AvatarProbeEvidence, LiveProviderProbeReceipt, LiveProviderProbeValidationError,
    LlmProbeEvidence, ProbeUsage, RT0_LIVE_PROVIDER_PROBE_SCHEMA, SttProbeEvidence,
    validate_live_provider_probe,
};

pub use live_readiness::{
    RT0_LIVE_READINESS_SCHEMA, Rt0LatencyReadiness, Rt0LiveReadinessBlocker,
    Rt0LiveReadinessReport, Rt0QualityReadiness, Rt0ReadinessDecisionFlags,
    Rt0RuntimeProofFlags, derive_rt0_live_readiness,
};

pub use owner_golden::{OwnerGoldenError, evaluate_bound_owner_golden_suite};

pub use exit::{
    AcceptanceEvidence, ArtifactCheckEvidence, AutomatedEvidence, CheckStatus,
    ConversationEvidence, ConversationPairEvidence, CostEvidence, EvidenceOrigin, HumanDimensions,
    HumanEvaluationEvidence, KnownLimitationsEvidence, LatencyDistributionMillis, ParticipantRole,
    PrivacyPermissionEvidence, QualityEvidence, RT0_EXIT_EVIDENCE_SCHEMA, RT0_EXIT_REPORT_SCHEMA,
    RecordStatus, Rt0ExitEvidence, Rt0ExitEvidenceError, Rt0ExitFailureCode, Rt0ExitReport,
    evaluate_rt0_exit_evidence,
};
pub use exit_assembly::{Rt0ExitAssemblyError, Rt0ExitAssemblyInputs, assemble_rt0_exit_evidence};
pub use exit_context::{OwnerGoldenVerificationContext, Rt0ExitVerificationContext};
pub use exit_support::{
    Rt0ExitSupportingArtifacts, evaluate_verified_rt0_exit_evidence,
    validate_rt0_exit_supporting_artifacts,
};
pub use exit_validation::{
    Rt0RuntimeSupportingProjection, derive_rt0_runtime_supporting_projection,
    validate_rt0_conversation_attempt_artifact, validate_rt0_conversation_evidence_binding,
};
pub use session_binding::{
    BoundLabSessionEvidenceAggregate, LabSessionBindingError, RT0_OWNER_LAB_SESSION_BINDING_SCHEMA,
    bind_owner_lab_session_evidence,
};
pub use session_evidence::{
    LabAvSyncDiagnostic, LabAvSyncDiagnosticInput, LabAvSyncEvidence, LabAvSyncEvidenceInput,
    LabAvSyncReference, LabAvSyncTrackIssue, LabMediaEvidence, LabMediaEvidenceInput,
    LabMediaEvidenceKind, LabSessionAggregateError, LabSessionEvidenceAggregate,
    LabSessionEvidenceSnapshot, LabVoiceAttemptEvidence, LabVoiceAttemptStatus,
    RT0_AV_SYNC_SAMPLES_PER_REQUEST, RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE,
    RT0_OWNER_LAB_SESSION_AGGREGATE_SCHEMA, RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA,
    aggregate_owner_lab_session_evidence,
};
pub use session_types::{LabTextAttemptEvidence, LabTextAttemptStatus, SessionUsageEvidence};

pub use supporting_scaffold::{
    RT0_MANUAL_SUPPORTING_FILES, RT0_RUNTIME_SUPPORTING_FILES, rt0_manual_supporting_scaffold,
    rt0_runtime_supporting_scaffold,
};

pub use supporting_preflight::{
    RT0_SUPPORTING_PREFLIGHT_SCHEMA, Rt0SupportingArtifactDigests, Rt0SupportingPreflightArtifacts,
    Rt0SupportingPreflightError, Rt0SupportingPreflightReport, preflight_rt0_supporting_artifacts,
};
