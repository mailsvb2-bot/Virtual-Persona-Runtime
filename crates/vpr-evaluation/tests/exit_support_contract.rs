use vpr_evaluation::{
    Rt0ExitEvidence, Rt0ExitEvidenceError, Rt0ExitSupportingArtifacts, sha256_hex,
    validate_rt0_exit_supporting_artifacts,
};

const ARTIFACTS: [&[u8]; 10] = [
    b"ci",
    b"e2e",
    b"owner",
    b"visitor",
    b"acceptance",
    b"quality",
    b"cost",
    b"privacy",
    b"human",
    b"limitations",
];

fn evidence() -> Rt0ExitEvidence {
    let mut evidence: Rt0ExitEvidence = serde_json::from_str(include_str!(
        "../../../docs/evaluation/rt0_exit_evidence.synthetic.example.json"
    ))
    .unwrap();
    evidence.automated.ci.artifact_sha256 = sha256_hex(ARTIFACTS[0]);
    evidence.automated.e2e.artifact_sha256 = sha256_hex(ARTIFACTS[1]);
    evidence.conversations.owner.artifact_sha256 = sha256_hex(ARTIFACTS[2]);
    evidence.conversations.visitor.artifact_sha256 = sha256_hex(ARTIFACTS[3]);
    evidence.acceptance.artifact_sha256 = sha256_hex(ARTIFACTS[4]);
    evidence.quality.artifact_sha256 = sha256_hex(ARTIFACTS[5]);
    evidence.cost.artifact_sha256 = sha256_hex(ARTIFACTS[6]);
    evidence.privacy_permissions.artifact_sha256 = sha256_hex(ARTIFACTS[7]);
    evidence.human_evaluation.artifact_sha256 = sha256_hex(ARTIFACTS[8]);
    evidence.known_limitations.document_sha256 = sha256_hex(ARTIFACTS[9]);
    evidence
}

fn artifacts() -> Rt0ExitSupportingArtifacts<'static> {
    Rt0ExitSupportingArtifacts {
        ci: ARTIFACTS[0],
        e2e: ARTIFACTS[1],
        owner_conversation: ARTIFACTS[2],
        visitor_conversation: ARTIFACTS[3],
        acceptance: ARTIFACTS[4],
        quality: ARTIFACTS[5],
        cost: ARTIFACTS[6],
        privacy_permissions: ARTIFACTS[7],
        human_evaluation: ARTIFACTS[8],
        known_limitations: ARTIFACTS[9],
    }
}

#[test]
fn exact_supporting_artifact_bytes_are_accepted() {
    assert_eq!(
        validate_rt0_exit_supporting_artifacts(&evidence(), artifacts()),
        Ok(())
    );
}

#[test]
fn every_supporting_artifact_digest_is_fail_closed() {
    for index in 0..10 {
        let mut evidence = evidence();
        let mismatch = sha256_hex(format!("different-{index}").as_bytes());
        match index {
            0 => evidence.automated.ci.artifact_sha256 = mismatch,
            1 => evidence.automated.e2e.artifact_sha256 = mismatch,
            2 => evidence.conversations.owner.artifact_sha256 = mismatch,
            3 => evidence.conversations.visitor.artifact_sha256 = mismatch,
            4 => evidence.acceptance.artifact_sha256 = mismatch,
            5 => evidence.quality.artifact_sha256 = mismatch,
            6 => evidence.cost.artifact_sha256 = mismatch,
            7 => evidence.privacy_permissions.artifact_sha256 = mismatch,
            8 => evidence.human_evaluation.artifact_sha256 = mismatch,
            9 => evidence.known_limitations.document_sha256 = mismatch,
            _ => unreachable!(),
        }
        assert_eq!(
            validate_rt0_exit_supporting_artifacts(&evidence, artifacts()),
            Err(Rt0ExitEvidenceError::InvalidArtifactDigest),
            "supporting artifact {index} was not fail-closed",
        );
    }
}
