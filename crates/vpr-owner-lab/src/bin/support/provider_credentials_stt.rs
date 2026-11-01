use std::error::Error;
use vpr_owner_lab::{load_provider_profile, save_provider_profile};

use super::{print_safe_profile, prompt_secret};

#[cfg(windows)]
pub(super) fn set_stt_profile() -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut profile = load_provider_profile()?
        .ok_or("secure VPR profile is missing; configure providers first")?;
    if profile.stt_provider != "deepgram" {
        return Err("set-stt currently supports only the existing Deepgram profile".into());
    }
    println!("Replace ONLY Deepgram API key; D-ID, LLM and TTS are preserved.");
    let key = prompt_secret("New Deepgram API key (hidden): ")?;
    if key.is_empty() || key.trim() != key || key.chars().any(char::is_control) {
        return Err("invalid Deepgram key format".into());
    }
    profile.stt_api_key = key;
    profile.validate()?;
    save_provider_profile(&profile)?;
    println!("Deepgram credential replaced in Windows Credential Manager.");
    print_safe_profile(&profile);
    Ok(())
}

