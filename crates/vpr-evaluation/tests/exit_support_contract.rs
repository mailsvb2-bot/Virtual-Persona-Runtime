use serde::Serialize;
use serde_json::{Value, json};
use vpr_evaluation::{
    Rt0ExitEvidence, Rt0ExitEvidenceError, Rt0ExitSupportingArtifacts, sha256_hex,
    validate_rt0_exit_supporting_artifacts,
};

#[derive(Clone)]
struct SupportingFixture {
    ci: Vec<u8>,
    e2e: Vec<u8>,
    owner: Vec<u8>,
    visitor: Vec<u8>,
    acceptance: Vec<u8>,
    quality: Vec<u8>,
    cost: Vec<u8>,
    privacy: Vec<u8>,
    human: Vec<u8>,
    limitations: Vec<u8>,
}

impl SupportingFixture {
    fn bind(evidence: &mut Rt0ExitEvidence) -> Self {
        let fixture = Self {
            ci: automated_claim_bytes(&evidence.automated.ci, &evidence.candidate_sha),
            e2e: automated_claim_bytes(&evidence.automated.e2e, &evidence.candidate_sha),
            owner: claim_bytes(&evidence.conversations.owner),
            visitor: claim_bytes(&evidence.conversations.visitor),
            acceptance: claim_bytes(&evidence.acceptance),
            quality: claim_bytes(&evidence.quality),
            cost: claim_bytes(&evidence.cost),
            privacy: claim_bytes(&evidence.privacy_permissions),
            human: claim_bytes(&evidence.human_evaluation),
            limitations: b"reviewed RT0 limitations\n".to_vec(),
        };
        for index in 0..10 {
            set_digest(evidence, index, sha256_hex(fixture.bytes(index)));
        }
        fixture
    }

    fn as_verification(&self) -> Rt0ExitSupportingArtifacts<'_> {
        Rt0ExitSupportingArtifacts {
            ci: &self.ci,
            e2e: &self.e2e,
            owner_conversation: &self.owner,
            visitor_conversation: &self.visitor,
            acceptance: &self.acceptance,
            quality: &self.quality,
            cost: &self.cost,
            privacy_permissions: &self.privacy,
            human_evaluation: &self.human,
            known_limitations: &self.limitations,
        }
    }

    fn bytes(&self, index: usize) -> &[u8] {
        match index {
            0 => &self.ci,
            1 => &self.e2e,
            2 => &self.owner,
            3 => &self.visitor,
            4 => &self.acceptance,
            5 => &self.quality,
            6 => &self.cost,
            7 => &self.privacy,
            8 => &self.human,
            9 => &self.limitations,
            _ => unreachable!(),
        }
    }

    fn bytes_mut(&mut self, index: usize) -> &mut Vec<u8> {
        match index {
            0 => &mut self.ci,
            1 => &mut self.e2e,
            2 => &mut self.owner,
            3 => &mut self.visitor,
            4 => &mut self.acceptance,
            5 => &mut self.quality,
            6 => &mut self.cost,
            7 => &mut self.privacy,
            8 => &mut self.human,
            9 => &mut self.limitations,
            _ => unreachable!(),
        }
    }
}

fn evidence() -> Rt0ExitEvidence {
    serde_json::from_str(include_str!(
        "../../../docs/evaluation/rt0_exit_evidence.synthetic.example.json"
    ))
    .unwrap()
}

fn claim_bytes<T: Serialize>(claim: &T) -> Vec<u8> {
    let mut value = serde_json::to_value(claim).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("artifact_sha256")
        .unwrap();
    serde_json::to_vec_pretty(&value).unwrap()
}

fn automated_claim_bytes<T: Serialize>(claim: &T, candidate_sha: &str) -> Vec<u8> {
    let mut value: Value = serde_json::from_slice(&claim_bytes(claim)).unwrap();
    value["candidate_sha"] = json!(candidate_sha);
    serde_json::to_vec_pretty(&value).unwrap()
}

fn set_digest(evidence: &mut Rt0ExitEvidence, index: usize, digest: String) {
    match index {
        0 => evidence.automated.ci.artifact_sha256 = digest,
        1 => evidence.automated.e2e.artifact_sha256 = digest,
        2 => evidence.conversations.owner.artifact_sha256 = digest,
        3 => evidence.conversations.visitor.artifact_sha256 = digest,
        4 => evidence.acceptance.artifact_sha256 = digest,
        5 => evidence.quality.artifact_sha256 = digest,
        6 => evidence.cost.artifact_sha256 = digest,
        7 => evidence.privacy_permissions.artifact_sha256 = digest,
        8 => evidence.human_evaluation.artifact_sha256 = digest,
        9 => evidence.known_limitations.document_sha256 = digest,
        _ => unreachable!(),
    }
}

#[test]
fn exact_supporting_claims_and_bytes_are_accepted() {
    let mut evidence = evidence();
    let fixture = SupportingFixture::bind(&mut evidence);
    assert_eq!(
        validate_rt0_exit_supporting_artifacts(&evidence, fixture.as_verification()),
        Ok(())
    );
}

#[test]
fn every_supporting_artifact_digest_is_fail_closed() {
    let mut evidence = evidence();
    let fixture = SupportingFixture::bind(&mut evidence);
    for index in 0..10 {
        let mut mismatched = evidence.clone();
        set_digest(
            &mut mismatched,
            index,
            sha256_hex(format!("different-{index}").as_bytes()),
        );
        assert_eq!(
            validate_rt0_exit_supporting_artifacts(&mismatched, fixture.as_verification()),
            Err(Rt0ExitEvidenceError::InvalidArtifactDigest),
            "supporting artifact {index} digest was not fail-closed",
        );
    }
}

#[test]
fn every_json_claim_rejects_rehashed_but_detached_content() {
    for index in 0..9 {
        let mut evidence = evidence();
        let mut fixture = SupportingFixture::bind(&mut evidence);
        let bytes = fixture.bytes_mut(index);
        let mut detached: Value = serde_json::from_slice(bytes).unwrap();
        detached
            .as_object_mut()
            .unwrap()
            .insert("detached_claim".into(), json!(true));
        *bytes = serde_json::to_vec_pretty(&detached).unwrap();
        set_digest(&mut evidence, index, sha256_hex(bytes));

        assert_eq!(
            validate_rt0_exit_supporting_artifacts(&evidence, fixture.as_verification()),
            Err(Rt0ExitEvidenceError::InvalidArtifactDigest),
            "supporting JSON claim {index} accepted detached rehashed content",
        );
    }
}

#[test]
fn malformed_rehashed_json_claim_is_rejected() {
    let mut evidence = evidence();
    let mut fixture = SupportingFixture::bind(&mut evidence);
    fixture.quality = b"not-json".to_vec();
    set_digest(&mut evidence, 5, sha256_hex(&fixture.quality));
    assert_eq!(
        validate_rt0_exit_supporting_artifacts(&evidence, fixture.as_verification()),
        Err(Rt0ExitEvidenceError::InvalidArtifactDigest)
    );
}

#[test]
fn automated_claim_candidate_binding_is_fail_closed_even_after_rehash() {
    for candidate_sha in ["2".repeat(40), "malformed".into()] {
        for index in 0..2 {
            let mut evidence = evidence();
            let mut fixture = SupportingFixture::bind(&mut evidence);
            let bytes = fixture.bytes_mut(index);
            let mut detached: Value = serde_json::from_slice(bytes).unwrap();
            detached["candidate_sha"] = json!(candidate_sha);
            *bytes = serde_json::to_vec_pretty(&detached).unwrap();
            set_digest(&mut evidence, index, sha256_hex(bytes));

            assert_eq!(
                validate_rt0_exit_supporting_artifacts(&evidence, fixture.as_verification()),
                Err(Rt0ExitEvidenceError::InvalidArtifactDigest),
                "automated supporting claim {index} accepted detached candidate binding",
            );
        }
    }
}
