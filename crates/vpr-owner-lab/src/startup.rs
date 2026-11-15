use super::{
    AppState, DEFAULT_PORT, HTTP_WORKERS, LabSessionEvidenceRecorder, ProviderBundle,
    engine_bootstrap, evidence_provenance, generate_csrf_token, handle_request, http_evidence,
    http_owner_capture, http_proxy, http_references, http_rt1, http_voice, launch,
    restore_reviewed_persona, rt1_startup,
};
use parking_lot::Mutex as ParkingMutex;
use std::env;
use std::error::Error;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};
use std::thread;

pub(super) fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let launch_options = launch::parse_launch_options(env::args().skip(1))
        .map_err(|error| format!("invalid owner-lab launch options: {error}"))?;
    if launch_options.show_help {
        launch::print_usage();
        return Ok(());
    }
    let egress_env = env::var("VPR_OWNER_LAB_ALLOW_EGRESS").ok();
    let egress_enabled = launch::resolve_egress_enabled(&launch_options, egress_env.as_deref());
    let rt0_evidence_mode = match env::var("VPR_OWNER_LAB_RT0_EVIDENCE") {
        Ok(value) if value.eq_ignore_ascii_case("true") => true,
        Ok(value) if value.eq_ignore_ascii_case("false") => false,
        Ok(_) => return Err("VPR_OWNER_LAB_RT0_EVIDENCE must be true or false".into()),
        Err(env::VarError::NotPresent) => false,
        Err(env::VarError::NotUnicode(_)) => {
            return Err("VPR_OWNER_LAB_RT0_EVIDENCE must be valid text".into());
        }
    };
    let port = env::var("VPR_OWNER_LAB_PORT")
        .ok()
        .map(|value| value.parse::<u16>())
        .transpose()?
        .unwrap_or(DEFAULT_PORT);
    let providers = ProviderBundle::from_env(false)?;
    let evidence_provenance = evidence_provenance::resolve(&providers, rt0_evidence_mode)?;
    let voice_profile = providers.voice_profile.clone();
    let _persona_store_lease = vpr_owner_lab::acquire_persona_store_lease()?;
    let mut engine = engine_bootstrap::assemble(providers, egress_enabled)?;
    if restore_reviewed_persona(&mut engine)? {
        println!("Restored reviewed Persona from persistent store.");
    }
    rt1_startup::recover_before_listener()?;
    let voice_playback = engine.voice_playback_registry();
    let mut evidence_recorder = LabSessionEvidenceRecorder::default();
    if let Some((candidate_sha, provider_state_sha256)) = evidence_provenance {
        evidence_recorder
            .bind_provenance(&candidate_sha, &provider_state_sha256)
            .map_err(|_| "owner-lab evidence provenance rejected")?;
        if rt0_evidence_mode {
            println!(
                "RT0 evidence provenance: candidate={candidate_sha} provider_state_sha256={provider_state_sha256}"
            );
        }
    }
    let state = Arc::new(AppState {
        engine: Mutex::new(engine),
        rt1: http_rt1::Rt1HttpState::default(),
        rt0_evidence_mode,
        voice_profile,
        owner_capture: http_owner_capture::OwnerCaptureHttpState::default(),
        reference_intake: http_references::ReferenceIntakeState::default(),
        active_voice_interrupt: ParkingMutex::new(None),
        voice_busy: AtomicBool::new(false),
        voice_cancel_requested: AtomicBool::new(false),
        voice_inputs: http_voice::VoiceInputRegistry::default(),
        voice_streams: http_voice::VoiceStreamRegistry::default(),
        voice_playback,
        replay_source: ParkingMutex::new(None),
        session_end_requested: AtomicBool::new(false),
        session_revocation: ParkingMutex::new(None),
        backend_stop: ParkingMutex::new(None),
        active_session_sequence: AtomicU64::new(0),
        greeting_session_sequence: AtomicU64::new(0),
        evidence: ParkingMutex::new(evidence_recorder),
        evidence_export: http_evidence::EvidenceExportTracker::default(),
        csrf_token: generate_csrf_token()?,
        port,
    });
    let address = format!("127.0.0.1:{port}");
    // Own the public socket before tiny_http can hide it inside Request::respond.
    // The existing tiny_http application remains the single parser and router.
    let listener = TcpListener::bind(&address)?;
    let server = Arc::new(tiny_http::Server::http("127.0.0.1:0")?);
    let upstream = server
        .server_addr()
        .to_ip()
        .ok_or("owner-lab internal HTTP listener is not TCP")?;
    println!("VPR Owner Lab: http://{address}");
    if http_rt1::enabled() {
        http_rt1::spawn_cleanup_reconciler(&state);
    }
    if !egress_enabled {
        println!("External provider egress is disabled until VPR_OWNER_LAB_ALLOW_EGRESS=true");
    }
    let workers: Vec<_> = (0..HTTP_WORKERS)
        .map(|_| {
            let server = Arc::clone(&server);
            let state = Arc::clone(&state);
            thread::spawn(move || {
                for request in server.incoming_requests() {
                    handle_request(request, &state);
                }
            })
        })
        .collect();
    let proxy_state = Arc::clone(&state);
    let proxy = thread::spawn(move || http_proxy::serve(&listener, upstream, &proxy_state));
    // The public listener owns process lifetime. If it fails, return the
    // error instead of waiting forever on internal tiny_http consumers.
    proxy.join().map_err(|_| "owner-lab HTTP proxy failed")??;
    for worker in workers {
        worker.join().map_err(|_| "owner-lab HTTP worker failed")?;
    }
    Ok(())
}
