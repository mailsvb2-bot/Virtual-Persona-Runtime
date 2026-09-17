use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use vpr_evaluation::{RT0_PROVIDER_STATE_SCHEMA, sha256_hex};

const CANDIDATE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SUITE_BYTES: &[u8] = include_bytes!("../../../docs/evaluation/rt0_golden_minimum.json");
const RELEASE_SPEC_BYTES: &[u8] = include_bytes!("../../../docs/releases/RT0_RELEASE_SPEC.md");

struct TempFile(PathBuf);

impl TempFile {
    fn new(name: &str, bytes: &[u8]) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "vpr-evaluation-{}-{nonce}-{name}",
            std::process::id()
        ));
        fs::write(&path, bytes).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn provider_state_bytes() -> Vec<u8> {
    serde_json::to_vec_pretty(&json!({
        "schema_version": RT0_PROVIDER_STATE_SCHEMA,
        "providers": [
            {
                "role": "stt",
                "provider": "contract-stt",
                "model_or_representation": "contract-v1",
                "configuration_fingerprint_sha256": sha256_hex(b"stt-config")
            },
            {
                "role": "llm",
                "provider": "contract-llm",
                "model_or_representation": "contract-v1",
                "configuration_fingerprint_sha256": sha256_hex(b"llm-config")
            },
            {
                "role": "tts",
                "provider": "contract-tts",
                "model_or_representation": "contract-v1",
                "configuration_fingerprint_sha256": sha256_hex(b"tts-config")
            },
            {
                "role": "avatar",
                "provider": "contract-avatar",
                "model_or_representation": "contract-v1",
                "configuration_fingerprint_sha256": sha256_hex(b"avatar-config")
            }
        ]
    }))
    .unwrap()
}

fn evidence_bytes(provider_bytes: &[u8]) -> Vec<u8> {
    serde_json::to_vec_pretty(&json!({
        "binding": {
            "schema_version": "rt0-evidence-binding-0.1",
            "candidate_sha": CANDIDATE,
            "release_spec_sha256": sha256_hex(RELEASE_SPEC_BYTES),
            "suite_sha256": sha256_hex(SUITE_BYTES),
            "provider_state_sha256": sha256_hex(provider_bytes)
        },
        "observations": []
    }))
    .unwrap()
}

fn run(candidate: &str) -> std::process::Output {
    let provider_bytes = provider_state_bytes();
    let suite = TempFile::new("suite.json", SUITE_BYTES);
    let evidence = TempFile::new("evidence.json", &evidence_bytes(&provider_bytes));
    let spec = TempFile::new("release.md", RELEASE_SPEC_BYTES);
    let providers = TempFile::new("provider-state.json", &provider_bytes);
    Command::new(env!("CARGO_BIN_EXE_vpr-evaluation"))
        .args([suite.path(), evidence.path(), spec.path(), providers.path()])
        .arg(candidate)
        .output()
        .unwrap()
}

#[test]
fn cli_accepts_exact_binding_but_fails_incomplete_golden_observations() {
    let output = run(CANDIDATE);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(CANDIDATE));
    assert!(stdout.contains("contract-avatar"));
    assert!(stdout.contains("MISSING_OBSERVATION"));
    assert!(!stdout.contains("prompt_ru"));
    assert!(!stdout.contains("response_text"));
}

#[test]
fn cli_rejects_candidate_mismatch_before_golden_evaluation() {
    let output = run("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("CANDIDATE_SHA_MISMATCH"));
    assert!(!stderr.contains("contract-stt"));
}
