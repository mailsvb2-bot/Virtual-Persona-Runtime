mod support;

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use vpr_evaluation::BoundGoldenReport;

const CANDIDATE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "vpr-owner-golden-cli-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_vpr-rt0-owner-golden")
}

#[test]
fn owner_golden_cli_writes_exact_bound_report() {
    let dir = TempDir::new();
    let release_spec = b"rt0 release spec";
    let baseline = support::fixture(release_spec, CANDIDATE);
    let owner = support::owner_fixture(
        release_spec,
        CANDIDATE,
        &baseline.provider_state,
        &baseline.provider_state_bytes,
    );

    let suite = dir.0.join("owner-golden-suite.json");
    let evidence = dir.0.join("owner-golden-evidence.json");
    let spec = dir.0.join("release-spec.md");
    let provider = dir.0.join("provider-state.json");
    let output = dir.0.join("owner-golden-report.json");

    fs::write(&suite, &owner.suite_bytes).unwrap();
    fs::write(&evidence, &owner.bundle_bytes).unwrap();
    fs::write(&spec, release_spec).unwrap();
    fs::write(&provider, &baseline.provider_state_bytes).unwrap();

    let result = Command::new(binary())
        .args([
            suite.as_os_str(),
            evidence.as_os_str(),
            spec.as_os_str(),
            provider.as_os_str(),
            output.as_os_str(),
            CANDIDATE.as_ref(),
        ])
        .output()
        .unwrap();

    assert!(result.status.success(), "stderr={}", String::from_utf8_lossy(&result.stderr));
    let actual: BoundGoldenReport = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(actual, owner.report);
}

#[test]
fn owner_golden_cli_rejects_public_baseline_suite() {
    let dir = TempDir::new();
    let release_spec = b"rt0 release spec";
    let baseline = support::fixture(release_spec, CANDIDATE);

    let suite = dir.0.join("public-suite.json");
    let evidence = dir.0.join("evidence.json");
    let spec = dir.0.join("release-spec.md");
    let provider = dir.0.join("provider-state.json");
    let output = dir.0.join("owner-golden-report.json");

    fs::write(&suite, support::SUITE_BYTES).unwrap();
    fs::write(&evidence, &baseline.bundle_bytes).unwrap();
    fs::write(&spec, release_spec).unwrap();
    fs::write(&provider, &baseline.provider_state_bytes).unwrap();

    let result = Command::new(binary())
        .args([
            suite.as_os_str(),
            evidence.as_os_str(),
            spec.as_os_str(),
            provider.as_os_str(),
            output.as_os_str(),
            CANDIDATE.as_ref(),
        ])
        .output()
        .unwrap();

    assert_eq!(result.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("OWNER_GOLDEN_SUITE_INVALID")
    );
    assert!(!output.exists());
}

#[test]
fn owner_golden_cli_never_overwrites_existing_report() {
    let dir = TempDir::new();
    let release_spec = b"rt0 release spec";
    let baseline = support::fixture(release_spec, CANDIDATE);
    let owner = support::owner_fixture(
        release_spec,
        CANDIDATE,
        &baseline.provider_state,
        &baseline.provider_state_bytes,
    );

    let suite = dir.0.join("owner-golden-suite.json");
    let evidence = dir.0.join("owner-golden-evidence.json");
    let spec = dir.0.join("release-spec.md");
    let provider = dir.0.join("provider-state.json");
    let output = dir.0.join("owner-golden-report.json");

    fs::write(&suite, &owner.suite_bytes).unwrap();
    fs::write(&evidence, &owner.bundle_bytes).unwrap();
    fs::write(&spec, release_spec).unwrap();
    fs::write(&provider, &baseline.provider_state_bytes).unwrap();
    fs::write(&output, b"existing").unwrap();

    let result = Command::new(binary())
        .args([
            suite.as_os_str(),
            evidence.as_os_str(),
            spec.as_os_str(),
            provider.as_os_str(),
            output.as_os_str(),
            CANDIDATE.as_ref(),
        ])
        .output()
        .unwrap();

    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("OUTPUT_EXISTS"));
    assert_eq!(fs::read(output).unwrap(), b"existing");
}
