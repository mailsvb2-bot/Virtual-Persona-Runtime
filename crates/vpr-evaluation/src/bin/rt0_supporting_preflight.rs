use std::{env, fs, path::Path};

use serde::Serialize;
use vpr_evaluation::{
    Rt0SupportingPreflightArtifacts, Rt0SupportingPreflightError,
    preflight_rt0_supporting_artifacts,
};

#[derive(Serialize)]
struct CliError<T: Serialize> {
    ok: bool,
    code: T,
}

struct SupportingArtifactBytes {
    ci: Vec<u8>,
    e2e: Vec<u8>,
    owner_conversation: Vec<u8>,
    visitor_conversation: Vec<u8>,
    acceptance: Vec<u8>,
    quality: Vec<u8>,
    cost: Vec<u8>,
    privacy_permissions: Vec<u8>,
    human_evaluation: Vec<u8>,
    known_limitations: Vec<u8>,
}

impl SupportingArtifactBytes {
    fn read(root: &Path) -> Result<Self, i32> {
        if !root.is_dir() {
            return input_invalid();
        }
        Ok(Self {
            ci: read_path(&root.join("ci-evidence.json"))?,
            e2e: read_path(&root.join("e2e-evidence.json"))?,
            owner_conversation: read_path(&root.join("owner-conversation.json"))?,
            visitor_conversation: read_path(&root.join("visitor-conversation.json"))?,
            acceptance: read_path(&root.join("acceptance.json"))?,
            quality: read_path(&root.join("quality.json"))?,
            cost: read_path(&root.join("cost.json"))?,
            privacy_permissions: read_path(&root.join("privacy-permissions.json"))?,
            human_evaluation: read_path(&root.join("human-evaluation.json"))?,
            known_limitations: read_path(&root.join("known-limitations.md"))?,
        })
    }

    fn as_preflight(&self) -> Rt0SupportingPreflightArtifacts<'_> {
        Rt0SupportingPreflightArtifacts {
            ci: &self.ci,
            e2e: &self.e2e,
            owner_conversation: &self.owner_conversation,
            visitor_conversation: &self.visitor_conversation,
            acceptance: &self.acceptance,
            quality: &self.quality,
            cost: &self.cost,
            privacy_permissions: &self.privacy_permissions,
            human_evaluation: &self.human_evaluation,
            known_limitations: &self.known_limitations,
        }
    }
}

fn main() {
    if let Err(code) = run() {
        std::process::exit(code);
    }
}

fn run() -> Result<(), i32> {
    let args: Vec<String> = env::args().skip(1).collect();
    let [supporting_dir, provider_state_path, candidate_sha] = args.as_slice() else {
        eprintln!(
            "usage: vpr-rt0-supporting-preflight <supporting-evidence-dir> <provider-state.json> <exact-candidate-sha>"
        );
        return Err(2);
    };
    let artifacts = SupportingArtifactBytes::read(Path::new(supporting_dir))?;
    let provider_state = read_path(Path::new(provider_state_path))?;
    match preflight_rt0_supporting_artifacts(
        artifacts.as_preflight(),
        &provider_state,
        candidate_sha,
    ) {
        Ok(report) => {
            println!("{}", serde_json::to_string_pretty(&report).map_err(|_| 2)?);
            Ok(())
        }
        Err(code) => {
            emit_error(code)?;
            Err(2)
        }
    }
}

fn read_path(path: &Path) -> Result<Vec<u8>, i32> {
    fs::read(path).map_err(|_| {
        let _ = emit_error("INPUT_INVALID");
        2
    })
}

fn input_invalid<T>() -> Result<T, i32> {
    let _ = emit_error("INPUT_INVALID");
    Err(2)
}

fn emit_error<T: Serialize>(code: T) -> Result<(), i32> {
    eprintln!(
        "{}",
        serde_json::to_string(&CliError { ok: false, code }).map_err(|_| 2)?
    );
    Ok(())
}

#[allow(dead_code)]
fn _error_contract(_: Rt0SupportingPreflightError) {}
