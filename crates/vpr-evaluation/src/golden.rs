use std::collections::{HashMap, HashSet};
use std::fmt::{Debug, Formatter};

use serde::{Deserialize, Serialize};
pub const RT0_GOLDEN_SCHEMA: &str = "rt0-golden-0.1";

use vpr_domain::{
    ClaimKind, DerivationKind, OwnerClaim, SourceKind, VerificationState, VerifiedOwnerOpinion,
};

#[derive(Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GoldenSuite {
    pub schema_version: String,
    pub suite_id: String,
    pub cases: Vec<GoldenCase>,
}

#[derive(Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GoldenCase {
    pub id: String,
    pub actor: GoldenActor,
    pub prompt_ru: String,
    pub expectations: Vec<GoldenExpectation>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoldenActor {
    Owner,
    Visitor,
}

#[derive(Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GoldenExpectation {
    ResponseContainsAll { tokens: Vec<String> },
    OwnerAttribution { eligible: bool },
    NoPrivateSentinels { sentinels: Vec<String> },
    ReasonCode { code: String },
    UnplayedTailNotSpoken,
    PersonaIdentityStable,
}

#[derive(Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GoldenObservation {
    pub case_id: String,
    pub response_text: Option<String>,
    pub owner_attribution: Option<AttributionEvidence>,
    pub reason_code: Option<String>,
    pub unplayed_tail_spoken: Option<bool>,
    pub persona_id_before: Option<String>,
    pub persona_id_after: Option<String>,
}

impl Debug for GoldenObservation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GoldenObservation")
            .field("case_id", &self.case_id)
            .field(
                "response_text",
                &self.response_text.as_ref().map(|_| "<redacted>"),
            )
            .field("owner_attribution", &self.owner_attribution)
            .field("reason_code", &self.reason_code)
            .field("unplayed_tail_spoken", &self.unplayed_tail_spoken)
            .field("persona_id_before", &self.persona_id_before)
            .field("persona_id_after", &self.persona_id_after)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AttributionEvidence {
    pub source: SourceEvidence,
    pub verification: VerificationEvidence,
    pub derivation: DerivationEvidence,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceEvidence {
    Owner,
    User,
    Document,
    Web,
    Tool,
    Model,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationEvidence {
    Unverified,
    Corroborated,
    OwnerVerified,
    SourceVerified,
    Disputed,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DerivationEvidence {
    Direct,
    Remembered,
    Inferred,
    Summarized,
    Simulated,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GoldenFailureCode {
    MissingObservation,
    MissingResponseText,
    MissingRequiredText,
    FalseOwnerAttribution,
    RequiredOwnerAttributionMissing,
    PrivateContextLeak,
    MissingReasonCode,
    WrongReasonCode,
    MissingSpokenCheckpoint,
    CancelledOutputMarkedSpoken,
    MissingPersonaIdentityEvidence,
    PersonaIdentityDrift,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GoldenCaseResult {
    pub case_id: String,
    pub passed: bool,
    pub failures: Vec<GoldenFailureCode>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GoldenReport {
    pub schema_version: String,
    pub suite_id: String,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub cases: Vec<GoldenCaseResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoldenSuiteError {
    UnsupportedSchemaVersion,
    BlankSchemaVersion,
    BlankSuiteId,
    EmptySuite,
    BlankCaseId,
    DuplicateCaseId,
    BlankPrompt,
    EmptyExpectations,
    BlankExpectationValue,
    DuplicateObservation,
    UnknownObservationCase,
}

/// Evaluates one Golden Set observation bundle without echoing raw responses into the report.
///
/// # Errors
/// Returns `GoldenSuiteError` when the suite or observation index is structurally invalid.
pub fn evaluate_golden_suite(
    suite: &GoldenSuite,
    observations: &[GoldenObservation],
) -> Result<GoldenReport, GoldenSuiteError> {
    validate_suite(suite)?;
    let observations = index_observations(suite, observations)?;
    let cases: Vec<_> = suite
        .cases
        .iter()
        .map(|case| evaluate_case(case, observations.get(case.id.as_str()).copied()))
        .collect();
    let passed = cases.iter().filter(|case| case.passed).count();
    Ok(GoldenReport {
        schema_version: suite.schema_version.clone(),
        suite_id: suite.suite_id.clone(),
        total: cases.len(),
        passed,
        failed: cases.len() - passed,
        cases,
    })
}

fn validate_suite(suite: &GoldenSuite) -> Result<(), GoldenSuiteError> {
    if suite.schema_version.trim().is_empty() {
        return Err(GoldenSuiteError::BlankSchemaVersion);
    }
    if suite.schema_version != RT0_GOLDEN_SCHEMA {
        return Err(GoldenSuiteError::UnsupportedSchemaVersion);
    }
    if suite.suite_id.trim().is_empty() {
        return Err(GoldenSuiteError::BlankSuiteId);
    }
    if suite.cases.is_empty() {
        return Err(GoldenSuiteError::EmptySuite);
    }
    let mut ids = HashSet::new();
    for case in &suite.cases {
        if case.id.trim().is_empty() {
            return Err(GoldenSuiteError::BlankCaseId);
        }
        if !ids.insert(case.id.as_str()) {
            return Err(GoldenSuiteError::DuplicateCaseId);
        }
        if case.prompt_ru.trim().is_empty() {
            return Err(GoldenSuiteError::BlankPrompt);
        }
        if case.expectations.is_empty() {
            return Err(GoldenSuiteError::EmptyExpectations);
        }
        for expectation in &case.expectations {
            validate_expectation(expectation)?;
        }
    }
    Ok(())
}

fn validate_expectation(expectation: &GoldenExpectation) -> Result<(), GoldenSuiteError> {
    let valid = match expectation {
        GoldenExpectation::ResponseContainsAll { tokens } => {
            !tokens.is_empty() && tokens.iter().all(|token| !token.trim().is_empty())
        }
        GoldenExpectation::NoPrivateSentinels { sentinels } => {
            !sentinels.is_empty() && sentinels.iter().all(|token| !token.trim().is_empty())
        }
        GoldenExpectation::ReasonCode { code } => !code.trim().is_empty(),
        GoldenExpectation::OwnerAttribution { .. }
        | GoldenExpectation::UnplayedTailNotSpoken
        | GoldenExpectation::PersonaIdentityStable => true,
    };
    if valid {
        Ok(())
    } else {
        Err(GoldenSuiteError::BlankExpectationValue)
    }
}

fn index_observations<'a>(
    suite: &GoldenSuite,
    observations: &'a [GoldenObservation],
) -> Result<HashMap<&'a str, &'a GoldenObservation>, GoldenSuiteError> {
    let valid_ids: HashSet<&str> = suite.cases.iter().map(|case| case.id.as_str()).collect();
    let mut indexed = HashMap::new();
    for observation in observations {
        if !valid_ids.contains(observation.case_id.as_str()) {
            return Err(GoldenSuiteError::UnknownObservationCase);
        }
        if indexed
            .insert(observation.case_id.as_str(), observation)
            .is_some()
        {
            return Err(GoldenSuiteError::DuplicateObservation);
        }
    }
    Ok(indexed)
}

fn evaluate_case(case: &GoldenCase, observation: Option<&GoldenObservation>) -> GoldenCaseResult {
    let Some(observation) = observation else {
        return GoldenCaseResult {
            case_id: case.id.clone(),
            passed: false,
            failures: vec![GoldenFailureCode::MissingObservation],
        };
    };
    let mut failures = Vec::new();
    for expectation in &case.expectations {
        evaluate_expectation(expectation, observation, &mut failures);
    }
    GoldenCaseResult {
        case_id: case.id.clone(),
        passed: failures.is_empty(),
        failures,
    }
}

fn evaluate_expectation(
    expectation: &GoldenExpectation,
    observation: &GoldenObservation,
    failures: &mut Vec<GoldenFailureCode>,
) {
    match expectation {
        GoldenExpectation::ResponseContainsAll { tokens } => {
            let Some(response) = observation.response_text.as_deref() else {
                failures.push(GoldenFailureCode::MissingResponseText);
                return;
            };
            let response = response.to_lowercase();
            if tokens
                .iter()
                .any(|token| !response.contains(&token.to_lowercase()))
            {
                failures.push(GoldenFailureCode::MissingRequiredText);
            }
        }
        GoldenExpectation::OwnerAttribution { eligible } => {
            evaluate_attribution(*eligible, observation.owner_attribution, failures);
        }
        GoldenExpectation::NoPrivateSentinels { sentinels } => {
            let Some(response) = observation.response_text.as_deref() else {
                failures.push(GoldenFailureCode::MissingResponseText);
                return;
            };
            let response = response.to_lowercase();
            if sentinels
                .iter()
                .any(|sentinel| response.contains(&sentinel.to_lowercase()))
            {
                failures.push(GoldenFailureCode::PrivateContextLeak);
            }
        }
        GoldenExpectation::ReasonCode { code } => match observation.reason_code.as_deref() {
            None => failures.push(GoldenFailureCode::MissingReasonCode),
            Some(actual) if actual != code => failures.push(GoldenFailureCode::WrongReasonCode),
            Some(_) => {}
        },
        GoldenExpectation::UnplayedTailNotSpoken => match observation.unplayed_tail_spoken {
            None => failures.push(GoldenFailureCode::MissingSpokenCheckpoint),
            Some(true) => failures.push(GoldenFailureCode::CancelledOutputMarkedSpoken),
            Some(false) => {}
        },
        GoldenExpectation::PersonaIdentityStable => {
            match (
                observation.persona_id_before.as_deref(),
                observation.persona_id_after.as_deref(),
            ) {
                (Some(before), Some(after)) if before == after => {}
                (Some(_), Some(_)) => failures.push(GoldenFailureCode::PersonaIdentityDrift),
                _ => failures.push(GoldenFailureCode::MissingPersonaIdentityEvidence),
            }
        }
    }
}

fn evaluate_attribution(
    expected_eligible: bool,
    evidence: Option<AttributionEvidence>,
    failures: &mut Vec<GoldenFailureCode>,
) {
    let Some(evidence) = evidence else {
        if expected_eligible {
            failures.push(GoldenFailureCode::RequiredOwnerAttributionMissing);
        }
        return;
    };
    let eligible = VerifiedOwnerOpinion::try_from(evidence.into_owner_claim()).is_ok();
    if eligible != expected_eligible {
        failures.push(if expected_eligible {
            GoldenFailureCode::RequiredOwnerAttributionMissing
        } else {
            GoldenFailureCode::FalseOwnerAttribution
        });
    }
}

impl AttributionEvidence {
    fn into_owner_claim(self) -> OwnerClaim {
        OwnerClaim {
            statement: "<evaluation-redacted>".into(),
            kind: ClaimKind::Opinion,
            source: self.source.into(),
            verification: self.verification.into(),
            derivation: self.derivation.into(),
        }
    }
}

impl From<SourceEvidence> for SourceKind {
    fn from(value: SourceEvidence) -> Self {
        match value {
            SourceEvidence::Owner => Self::Owner,
            SourceEvidence::User => Self::User,
            SourceEvidence::Document => Self::Document,
            SourceEvidence::Web => Self::Web,
            SourceEvidence::Tool => Self::Tool,
            SourceEvidence::Model => Self::Model,
        }
    }
}

impl From<VerificationEvidence> for VerificationState {
    fn from(value: VerificationEvidence) -> Self {
        match value {
            VerificationEvidence::Unverified => Self::Unverified,
            VerificationEvidence::Corroborated => Self::Corroborated,
            VerificationEvidence::OwnerVerified => Self::OwnerVerified,
            VerificationEvidence::SourceVerified => Self::SourceVerified,
            VerificationEvidence::Disputed => Self::Disputed,
        }
    }
}

impl From<DerivationEvidence> for DerivationKind {
    fn from(value: DerivationEvidence) -> Self {
        match value {
            DerivationEvidence::Direct => Self::Direct,
            DerivationEvidence::Remembered => Self::Remembered,
            DerivationEvidence::Inferred => Self::Inferred,
            DerivationEvidence::Summarized => Self::Summarized,
            DerivationEvidence::Simulated => Self::Simulated,
        }
    }
}
