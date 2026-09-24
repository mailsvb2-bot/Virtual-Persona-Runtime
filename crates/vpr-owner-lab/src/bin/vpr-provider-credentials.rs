use std::env;
use std::error::Error;

#[cfg(windows)]
use std::io::{self, Write};
#[cfg(windows)]
use vpr_integration::ProviderErrorKind;
#[cfg(windows)]
use vpr_owner_lab::{
    ProviderCredentialProfile, delete_provider_profile, load_provider_profile,
    save_provider_profile,
};
#[cfg(windows)]
use vpr_provider_did_agent_streams::{
    DidAgentStreamsAvatar, DidAgentStreamsConfig, DidRuntimeAccessFailure, DidRuntimeAccessProbe,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("provider-credentials failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let command = env::args().nth(1).unwrap_or_else(|| "status".into());
    if command == "status" {
        println!("Secure VPR provider profile: Windows-only");
        Ok(())
    } else {
        Err("persistent provider credential storage is available only on Windows".into())
    }
}

#[cfg(windows)]
fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let command = env::args().nth(1).unwrap_or_else(|| "status".into());
    match command.as_str() {
        "set" => set_profile(),
        "set-did" => set_did_profile(),
        "import-env" => import_env_profile(),
        "status" => status(),
        "probe-did" => probe_did(),
        "clear" => clear(),
        _ => Err(
            "usage: vpr-provider-credentials <set|set-did|import-env|status|probe-did|clear>"
                .into(),
        ),
    }
}

#[cfg(windows)]
fn set_profile() -> Result<(), Box<dyn Error + Send + Sync>> {
    println!("VPR RT0 provider setup: D-ID + Deepgram + DeepSeek");
    println!("Secrets are entered without echo and stored in Windows Credential Manager.");
    let did_api_key = prompt_secret("D-ID API key: ")?;
    let did_agent_id = prompt_line("D-ID agent ID: ")?;
    let deepgram_api_key = prompt_secret("Deepgram API key: ")?;
    let deepseek_api_key = prompt_secret("DeepSeek API key: ")?;

    let profile = ProviderCredentialProfile::canonical_rt0(
        &did_api_key,
        &did_agent_id,
        deepgram_api_key,
        deepseek_api_key,
    );
    probe_profile_did(&profile)?;
    save_provider_profile(&profile)?;
    println!("Saved securely for the current Windows user.");
    print_safe_profile(&profile);
    Ok(())
}

#[cfg(windows)]
fn set_did_profile() -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut profile = load_provider_profile()?
        .ok_or("secure VPR provider profile is not configured; run vpr-provider-credentials set")?;
    println!("Replace only D-ID credentials; Deepgram and DeepSeek credentials are preserved.");
    println!(
        "Enter the raw D-ID key as API_USERNAME:API_PASSWORD; an accidental Basic prefix is stripped."
    );
    let did_api_key = prompt_secret("D-ID API key: ")?;
    let did_agent_id = prompt_line("D-ID agent ID: ")?;
    profile.replace_did_credentials(&did_api_key, &did_agent_id);
    probe_profile_did(&profile)?;
    save_provider_profile(&profile)?;
    println!("Updated D-ID credentials securely for the current Windows user.");
    print_safe_profile(&profile);
    Ok(())
}

#[cfg(windows)]
fn import_env_profile() -> Result<(), Box<dyn Error + Send + Sync>> {
    require_canonical_provider_if_set("VPR_OWNER_LAB_STT_PROVIDER", "deepgram")?;
    require_canonical_provider_if_set("VPR_OWNER_LAB_LLM_PROVIDER", "deepseek")?;
    let did_api_key = required_process_value("VPR_DID_API_KEY")?;
    let did_agent_id = required_process_value("VPR_DID_AGENT_ID")?;
    let profile = ProviderCredentialProfile::canonical_rt0(
        &did_api_key,
        &did_agent_id,
        required_process_value("VPR_OWNER_LAB_STT_API_KEY")?,
        required_process_value("VPR_OWNER_LAB_LLM_API_KEY")?,
    );
    probe_profile_did(&profile)?;
    save_provider_profile(&profile)?;
    println!("Imported current CMD provider credentials into Windows Credential Manager.");
    print_safe_profile(&profile);
    Ok(())
}

#[cfg(windows)]
fn required_process_value(name: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("environment variable {name} is not set in this CMD").into())
}

#[cfg(windows)]
fn require_canonical_provider_if_set(
    name: &str,
    expected: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let Some(value) = env::var(name)
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    if value != expected {
        return Err(format!(
            "environment variable {name} selects {value}; import-env only migrates canonical RT0 provider {expected}"
        )
        .into());
    }
    Ok(())
}

#[cfg(windows)]
fn status() -> Result<(), Box<dyn Error + Send + Sync>> {
    match load_provider_profile()? {
        Some(profile) => {
            println!("Secure VPR provider profile: configured");
            print_safe_profile(&profile);
        }
        None => println!("Secure VPR provider profile: not configured"),
    }
    Ok(())
}

#[cfg(windows)]
fn probe_did() -> Result<(), Box<dyn Error + Send + Sync>> {
    let profile = load_provider_profile()?
        .ok_or("secure VPR provider profile is not configured; run vpr-provider-credentials set")?;
    probe_profile_did(&profile)
}

#[cfg(windows)]
fn probe_profile_did(
    profile: &ProviderCredentialProfile,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let provider = DidAgentStreamsAvatar::new(
        DidAgentStreamsConfig::new(
            profile.did_endpoint.clone(),
            profile.did_api_key.clone(),
            profile.did_agent_id.clone(),
        )
        .with_fluent(profile.did_fluent),
    )
    .map_err(|_| "D-ID provider configuration rejected")?;

    match provider.probe_runtime_access_detailed() {
        Ok(DidRuntimeAccessProbe::Presenter(presenter)) => {
            println!("D-ID credential probe: OK (presenter={presenter})");
            Ok(())
        }
        Ok(DidRuntimeAccessProbe::LegacyStreamFallback) => {
            println!(
                "D-ID credential probe: OK (legacy stream fallback; agent metadata GET returned 403)"
            );
            Ok(())
        }
        Err(DidRuntimeAccessFailure::Unauthorized) => Err(
            "D-ID credential probe: UNAUTHORIZED_401 (D-ID rejected the API key; use the raw API_USERNAME:API_PASSWORD value, without a Basic prefix)"
                .into(),
        ),
        Err(DidRuntimeAccessFailure::Forbidden) => Err(
            "D-ID credential probe: FORBIDDEN_403 (D-ID denied access to the configured agent or stream)"
                .into(),
        ),
        Err(DidRuntimeAccessFailure::Provider(error)) => {
            let message = match error.kind {
                ProviderErrorKind::PolicyDenied => "D-ID credential probe: POLICY_DENIED",
                ProviderErrorKind::RateLimited => "D-ID credential probe: RATE_LIMITED",
                ProviderErrorKind::Timeout => "D-ID credential probe: TIMEOUT",
                ProviderErrorKind::Unavailable => "D-ID credential probe: UNAVAILABLE",
                ProviderErrorKind::Cancelled => "D-ID credential probe: CANCELLED",
                ProviderErrorKind::InvalidResponse => "D-ID credential probe: INVALID_RESPONSE",
            };
            Err(message.into())
        }
    }
}

#[cfg(windows)]
fn clear() -> Result<(), Box<dyn Error + Send + Sync>> {
    delete_provider_profile()?;
    println!("Secure VPR provider profile cleared.");
    Ok(())
}

#[cfg(windows)]
fn prompt_secret(prompt: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
    let value = rpassword::prompt_password(prompt)?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err("secret value cannot be empty".into());
    }
    Ok(value)
}

#[cfg(windows)]
fn prompt_line(prompt: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err("value cannot be empty".into());
    }
    Ok(value)
}

#[cfg(windows)]
fn print_safe_profile(profile: &ProviderCredentialProfile) {
    println!(
        "  avatar: D-ID ({}) · agent={} · legacy fluent disabled",
        profile.did_endpoint,
        redact_identifier(&profile.did_agent_id)
    );
    println!(
        "  STT: {} {} ({})",
        profile.stt_provider, profile.stt_model, profile.stt_endpoint
    );
    println!(
        "  LLM: {} {} ({})",
        profile.llm_provider, profile.llm_model, profile.llm_endpoint
    );
    println!("  API keys: stored; values are never printed");
}

#[cfg(windows)]
fn redact_identifier(value: &str) -> String {
    let chars: Vec<char> = value.trim().chars().collect();
    if chars.len() <= 8 {
        return "***".into();
    }
    let prefix: String = chars.iter().take(4).collect();
    let suffix: String = chars.iter().rev().take(4).rev().collect();
    format!("{prefix}…{suffix}")
}
