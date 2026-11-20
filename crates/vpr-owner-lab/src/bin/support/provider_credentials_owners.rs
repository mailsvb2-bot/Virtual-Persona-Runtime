//! Operator-only Windows Credential Manager provisioning for a canonical RT1 owner.
use std::error::Error;
use vpr_owner_lab::{
    OwnerProviderProfile, ProviderCredentialProfile, StoredTtsCredential,
    delete_owner_provider_profile, save_owner_provider_profile,
};
use super::{prompt_line, prompt_secret};

pub(super) fn set_rt1_owner(owner_id: Option<&str>) -> Result<(), Box<dyn Error + Send + Sync>> {
    let owner_id = owner_id.ok_or("set-rt1-owner requires an existing canonical owner ID")?;
    // No public account creation here. This operator-only command stores
    // credentials for the canonical owner ID obtained from reviewed onboarding.
    if owner_id.len() != 32 || !owner_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid RT1 owner ID".into());
    }
    println!("Configure private RT1 worker and voice providers for this owner.");
    println!("All secrets stay in Windows Credential Manager for the current Windows user.");
    let worker_endpoint = prompt_line("Owner worker HTTPS endpoint: ")?;
    let worker_token = prompt_secret("Owner worker bearer (hidden): ")?;
    let stt_key = prompt_secret("Owner Deepgram STT API key (hidden): ")?;
    let llm_key = prompt_secret("Owner DeepSeek LLM API key (hidden): ")?;
    let mut profile = ProviderCredentialProfile::local_rt0(
        &worker_endpoint,
        &worker_token,
        stt_key,
        llm_key,
    );
    let tts = StoredTtsCredential {
        provider: prompt_line("Owner TTS provider (piper/openai-speech/elevenlabs): ")?
            .to_ascii_lowercase(),
        endpoint: prompt_line("Owner TTS endpoint: ")?,
        api_key: prompt_secret("Owner TTS API key (hidden): ")?,
        model: prompt_line("Owner TTS model: ")?,
        voice: prompt_line("Owner TTS voice: ")?,
    };
    profile.set_tts(tts)?;
    let scoped = OwnerProviderProfile::new(
        owner_id,
        profile,
        worker_endpoint,
        worker_token,
    )?;
    save_owner_provider_profile(owner_id, &scoped)?;
    println!("RT1 owner-specific credentials saved securely; no keys printed.");
    Ok(())
}

pub(super) fn clear_rt1_owner(owner_id: Option<&str>) -> Result<(), Box<dyn Error + Send + Sync>> {
    let owner_id = owner_id.ok_or("clear-rt1-owner requires a canonical owner ID")?;
    delete_owner_provider_profile(owner_id)?;
    println!("RT1 owner credentials revoked; other owners and RT0 unaffected.");
    Ok(())
}

