use std::{env, fs, process};

use vpr_evaluation::{
    BoundLabSessionEvidenceAggregate, bind_owner_lab_session_evidence, derive_rt0_live_readiness,
};

fn main() {
    if let Err(error) = run(&env::args().skip(1).collect::<Vec<_>>()) {
        eprintln!("{error}");
        process::exit(2);
    }
}

fn run(args: &[String]) -> Result<(), String> {
    if args.len() < 4 {
        return Err(
            "usage: vpr-rt0-live-readiness <provider-state.json> <bound-session-aggregate.json> \
             <exact-candidate-sha> <session-snapshot.json>..."
                .into(),
        );
    }

    let provider_state = read(&args[0], "provider state")?;
    let bound_bytes = read(&args[1], "bound session aggregate")?;
    let bound: BoundLabSessionEvidenceAggregate = serde_json::from_slice(&bound_bytes)
        .map_err(|_| "bound session aggregate JSON is invalid")?;
    let candidate = args[2].trim();
    let snapshots = args[3..]
        .iter()
        .map(|path| read(path, "session snapshot"))
        .collect::<Result<Vec<_>, _>>()?;
    let snapshot_refs = snapshots.iter().map(Vec::as_slice).collect::<Vec<_>>();

    let recomputed = bind_owner_lab_session_evidence(&snapshot_refs, &provider_state, candidate)
        .map_err(|error| format!("runtime evidence recomputation failed: {error:?}"))?;
    if recomputed != bound {
        return Err("bound session aggregate does not match the supplied raw snapshots".into());
    }

    let report = derive_rt0_live_readiness(&recomputed);
    println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|_| "live readiness report serialization failed")?
    );
    Ok(())
}

fn read(path: &str, label: &str) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|_| format!("{label} could not be read"))
}
