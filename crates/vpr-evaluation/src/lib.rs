mod binding;
mod exit;
mod golden;

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

pub use exit::{
    AcceptanceEvidence, ArtifactCheckEvidence, AutomatedEvidence, CheckStatus,
    ConversationEvidence, ConversationPairEvidence, CostEvidence, EvidenceOrigin, HumanDimensions,
    HumanEvaluationEvidence, KnownLimitationsEvidence, LatencyDistributionMillis, ParticipantRole,
    PrivacyPermissionEvidence, QualityEvidence, RT0_EXIT_EVIDENCE_SCHEMA, RT0_EXIT_REPORT_SCHEMA,
    RecordStatus, Rt0ExitEvidence, Rt0ExitEvidenceError, Rt0ExitFailureCode, Rt0ExitReport,
    Rt0ExitVerificationContext, evaluate_rt0_exit_evidence,
};
