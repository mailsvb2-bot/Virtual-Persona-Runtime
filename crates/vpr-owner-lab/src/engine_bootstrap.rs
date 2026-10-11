//! Startup-only wiring of configured provider ports into the single Owner Lab engine.
//! This module creates no additional state, policy authority or execution path.

use std::error::Error;

use vpr_owner_lab::{OwnerLabEngine, ProviderBundle};

pub(super) fn assemble(
    mut providers: ProviderBundle,
    egress_enabled: bool,
) -> Result<OwnerLabEngine, Box<dyn Error + Send + Sync>> {
    let mut engine = OwnerLabEngine::new(providers.avatar, egress_enabled)
        .map_err(|_| "owner-lab runtime initialization failed")?;
    if let Some(llm) = providers.llm.take() {
        engine = engine.with_llm(llm);
    }
    if let Some(stt) = providers.stt.take() {
        engine = engine.with_stt(stt);
    }
    if let Some(tts) = providers.tts.take() {
        let name = providers
            .tts_name
            .take()
            .ok_or("missing TTS provider name")?;
        engine = engine.with_named_tts(name, tts);
    }
    if let Some(tts) = providers.tts_fallback.take() {
        let name = providers
            .tts_fallback_name
            .take()
            .ok_or("missing alternate TTS provider name")?;
        engine = engine.with_tts_fallback(name, tts);
    }
    Ok(engine)
}
