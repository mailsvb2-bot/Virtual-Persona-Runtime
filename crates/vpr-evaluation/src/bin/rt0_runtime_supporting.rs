use std::{env, fs, path::Path, process};

use serde::Serialize;
use vpr_evaluation::{
    BoundLabSessionEvidenceAggregate, ConversationEvidence, QualityEvidence,
    derive_rt0_runtime_supporting_projection, sha256_hex,
};

const OUTPUTS: [&str; 3] = [
    "owner-conversation.json",
    "visitor-conversation.json",
    "quality.json",
];

#[derive(Serialize)]
struct ConversationSupportingClaim<'a> {
    origin: vpr_evaluation::EvidenceOrigin,
    role: vpr_evaluation::ParticipantRole,
    russian: vpr_evaluation::CheckStatus,
    voice: vpr_evaluation::CheckStatus,
    video: vpr_evaluation::CheckStatus,
    completed_turns: u32,
    interruption_exercised: vpr_evaluation::CheckStatus,
    candidate_sha: &'a str,
    provider_state_sha256: &'a str,
}

#[derive(Serialize)]
struct QualitySupportingClaim<'a> {
    origin: vpr_evaluation::EvidenceOrigin,
    text_first_meaningful_response: vpr_evaluation::LatencyDistributionMillis,
    first_meaningful_audio: vpr_evaluation::LatencyDistributionMillis,
    interruption_stop: vpr_evaluation::LatencyDistributionMillis,
    first_useful_video: vpr_evaluation::LatencyDistributionMillis,
    av_sync_absolute_offset: vpr_evaluation::LatencyDistributionMillis,
    recoverable_reconnect: vpr_evaluation::LatencyDistributionMillis,
    candidate_sha: &'a str,
    provider_state_sha256: &'a str,
}

fn main() {
    if let Err(error) = run(&env::args().skip(1).collect::<Vec<_>>()) {
        eprintln!("{error}");
        process::exit(2);
    }
}

fn run(args: &[String]) -> Result<(), String> {
    if args.len() < 6 {
        return Err(
            "usage: vpr-rt0-runtime-supporting <output-dir> <provider-state.json> \
             <conversation-attempt.json> <bound-session-aggregate.json> <candidate-sha> \
             <session-snapshot.json>..."
                .into(),
        );
    }

    let output_dir = Path::new(&args[0]);
    let provider_state = read(&args[1], "provider state")?;
    let conversation_attempt = read(&args[2], "conversation attempt")?;
    let bound_bytes = read(&args[3], "bound session aggregate")?;
    let candidate_sha = args[4].trim();
    let bound: BoundLabSessionEvidenceAggregate = serde_json::from_slice(&bound_bytes)
        .map_err(|_| "bound session aggregate JSON is invalid")?;

    let snapshots = args[5..]
        .iter()
        .map(|path| read(path, "session snapshot"))
        .collect::<Result<Vec<_>, _>>()?;
    let snapshot_refs = snapshots.iter().map(Vec::as_slice).collect::<Vec<_>>();

    let projection = derive_rt0_runtime_supporting_projection(
        &conversation_attempt,
        &bound,
        &snapshot_refs,
        &provider_state,
        candidate_sha,
    )
    .map_err(|error| format!("runtime supporting projection failed: {error:?}"))?;

    if output_dir.exists() && !output_dir.is_dir() {
        return Err("output path exists and is not a directory".into());
    }
    fs::create_dir_all(output_dir).map_err(|_| "output directory could not be created")?;
    if OUTPUTS.iter().any(|name| output_dir.join(name).exists()) {
        return Err("refusing to overwrite an existing runtime supporting-evidence file".into());
    }

    let provider_state_sha256 = sha256_hex(&provider_state);
    write_json(
        &output_dir.join("owner-conversation.json"),
        &conversation_claim(
            &projection.conversations.owner,
            candidate_sha,
            &provider_state_sha256,
        ),
    )?;
    write_json(
        &output_dir.join("visitor-conversation.json"),
        &conversation_claim(
            &projection.conversations.visitor,
            candidate_sha,
            &provider_state_sha256,
        ),
    )?;
    write_json(
        &output_dir.join("quality.json"),
        &quality_claim(&projection.quality, candidate_sha, &provider_state_sha256),
    )?;

    println!(
        "RT0 runtime supporting claims created at {}. \
         Only conversation and quality claims were derived; acceptance, privacy, cost and human \
         review remain separate real evidence.",
        output_dir.display()
    );
    Ok(())
}

fn conversation_claim<'a>(
    evidence: &ConversationEvidence,
    candidate_sha: &'a str,
    provider_state_sha256: &'a str,
) -> ConversationSupportingClaim<'a> {
    ConversationSupportingClaim {
        origin: evidence.origin,
        role: evidence.role,
        russian: evidence.russian,
        voice: evidence.voice,
        video: evidence.video,
        completed_turns: evidence.completed_turns,
        interruption_exercised: evidence.interruption_exercised,
        candidate_sha,
        provider_state_sha256,
    }
}

fn quality_claim<'a>(
    evidence: &QualityEvidence,
    candidate_sha: &'a str,
    provider_state_sha256: &'a str,
) -> QualitySupportingClaim<'a> {
    QualitySupportingClaim {
        origin: evidence.origin,
        text_first_meaningful_response: evidence.text_first_meaningful_response,
        first_meaningful_audio: evidence.first_meaningful_audio,
        interruption_stop: evidence.interruption_stop,
        first_useful_video: evidence.first_useful_video,
        av_sync_absolute_offset: evidence.av_sync_absolute_offset,
        recoverable_reconnect: evidence.recoverable_reconnect,
        candidate_sha,
        provider_state_sha256,
    }
}

fn read(path: &str, label: &str) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|_| format!("{label} could not be read"))
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|_| "JSON serialization failed")?;
    bytes.push(b'\n');
    fs::write(path, bytes).map_err(|_| "runtime supporting-evidence write failed".to_string())
}
