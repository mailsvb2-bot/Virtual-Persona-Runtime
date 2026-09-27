use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, process};

use serde::Serialize;
use vpr_evaluation::{
    EvidenceVerificationContext, GoldenEvidenceBundle, GoldenSuite, OwnerGoldenError,
    ProviderStateManifest, evaluate_bound_owner_golden_suite, sha256_hex,
};

#[derive(Serialize)]
struct CliError<T: Serialize> {
    ok: bool,
    code: T,
}

#[derive(Serialize)]
struct ReportReceipt<'a> {
    ok: bool,
    candidate_sha: &'a str,
    output_sha256: String,
}

fn main() {
    if let Err(code) = run() {
        process::exit(code);
    }
}

fn run() -> Result<(), i32> {
    let args: Vec<String> = env::args().skip(1).collect();
    let [
        suite_path,
        evidence_path,
        release_spec_path,
        provider_state_path,
        output_path,
        candidate_sha,
    ] = args.as_slice()
    else {
        eprintln!(
            "usage: vpr-rt0-owner-golden <owner-golden-suite.json> <owner-golden-evidence.json> <release-spec.md> <provider-state.json> <output-owner-golden-report.json> <exact-candidate-sha>"
        );
        return Err(2);
    };

    let output_path = Path::new(output_path);
    if output_path.exists() {
        return fail("OUTPUT_EXISTS");
    }

    let suite_bytes = read(Path::new(suite_path))?;
    let evidence_bytes = read(Path::new(evidence_path))?;
    let release_spec_bytes = read(Path::new(release_spec_path))?;
    let provider_state_bytes = read(Path::new(provider_state_path))?;

    let suite: GoldenSuite = parse(&suite_bytes)?;
    let evidence: GoldenEvidenceBundle = parse(&evidence_bytes)?;
    let provider_state: ProviderStateManifest = parse(&provider_state_bytes)?;

    let report = evaluate_bound_owner_golden_suite(
        &suite,
        &evidence,
        EvidenceVerificationContext {
            suite_bytes: &suite_bytes,
            release_spec_bytes: &release_spec_bytes,
            provider_state: &provider_state,
            provider_state_bytes: &provider_state_bytes,
            evidence_bytes: &evidence_bytes,
            exact_candidate_sha: candidate_sha,
        },
    )
    .map_err(|error| match error {
        OwnerGoldenError::SuiteNotOwnerSpecific => emit_code("OWNER_GOLDEN_SUITE_INVALID"),
        OwnerGoldenError::ReportMismatch => emit_code("OWNER_GOLDEN_REPORT_MISMATCH"),
        OwnerGoldenError::Binding(code) => emit_code(code),
    })?;

    let output_bytes = serde_json::to_vec_pretty(&report).map_err(|_| 2)?;
    write_new_atomic(output_path, &output_bytes)?;

    println!(
        "{}",
        serde_json::to_string_pretty(&ReportReceipt {
            ok: true,
            candidate_sha,
            output_sha256: sha256_hex(&output_bytes),
        })
        .map_err(|_| 2)?
    );

    if report.golden.failed == 0 {
        Ok(())
    } else {
        Err(1)
    }
}

fn read(path: &Path) -> Result<Vec<u8>, i32> {
    fs::read(path).map_err(|_| emit_code("INPUT_INVALID"))
}

fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, i32> {
    serde_json::from_slice(bytes).map_err(|_| emit_code("INPUT_INVALID"))
}

fn write_new_atomic(path: &Path, bytes: &[u8]) -> Result<(), i32> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return fail("OUTPUT_PATH_INVALID");
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| emit_code("OUTPUT_PATH_INVALID"))?;
    let temp_path = unique_temp_path(parent, file_name);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)
        .map_err(|_| emit_code("OUTPUT_TEMP_UNAVAILABLE"))?;
    if file.write_all(bytes).is_err() || file.sync_all().is_err() {
        drop(file);
        let _ = fs::remove_file(&temp_path);
        return fail("OUTPUT_WRITE_FAILED");
    }
    drop(file);
    if fs::hard_link(&temp_path, path).is_err() {
        let _ = fs::remove_file(&temp_path);
        return fail("OUTPUT_COMMIT_FAILED");
    }
    if fs::remove_file(&temp_path).is_err() {
        return fail("OUTPUT_TEMP_CLEANUP_FAILED");
    }
    Ok(())
}

fn unique_temp_path(parent: &Path, file_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    parent.join(format!(".{file_name}.{}.{}.tmp", process::id(), nanos))
}

fn emit_code<T: Serialize>(code: T) -> i32 {
    eprintln!(
        "{}",
        serde_json::to_string(&CliError { ok: false, code })
            .unwrap_or_else(|_| "{\"ok\":false,\"code\":\"INTERNAL_ERROR\"}".into())
    );
    2
}

fn fail<T>(code: &'static str) -> Result<T, i32> {
    Err(emit_code(code))
}
