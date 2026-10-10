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
#[cfg(windows)]
use vpr_provider_local_open_source::{LocalOpenSourceAvatar, LocalOpenSourceAvatarConfig};

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
        "set-local-avatar" => set_local_avatar(),
        "use-did-avatar" => use_did_avatar(),
        "import-env" => import_env_profile(),
        "status" => status(),
        "probe-did" => probe_did(),
        "probe-avatar" => probe_avatar(),
        "clear" => clear(),
        _ => Err(
            "usage: vpr-provider-credentials <set|set-did|set-local-avatar|use-did-avatar|import-env|status|probe-did|probe-avatar|clear>"
                .into(),
        ),
    }
}

#[cfg(windows)]
fn set_profile() -> Result<(), Box<dyn Error + Send + Sync>> {
    println!("VPR RT0 provider setup: choose an avatar provider + Deepgram + DeepSeek.");
    println!("Secrets are entered without echo and stored in Windows Credential Manager.");
    let avatar_provider =
        prompt_line("Avatar provider (did/local-open-source): ")?.to_ascii_lowercase();
    let profile = match avatar_provider.as_str() {
        "did" | "d-id" | "did-agent-streams" => {
            let did_api_key = prompt_secret("D-ID API key: ")?;
            let did_agent_id = prompt_line("D-ID agent ID: ")?;
            ProviderCredentialProfile::canonical_rt0(
                &did_api_key,
                &did_agent_id,
                prompt_secret("Deepgram API key: ")?,
                prompt_secret("DeepSeek API key: ")?,
            )
        }
        "local" | "local-open-source" => ProviderCredentialProfile::local_rt0(
            &prompt_line("Local avatar HTTPS endpoint: ")?,
            &prompt_secret("Local avatar API token: ")?,
            prompt_secret("Deepgram API key: ")?,
            prompt_secret("DeepSeek API key: ")?,
        ),
        _ => return Err("avatar provider must be did or local-open-source".into()),
    };

    probe_selected_avatar(&profile)?;
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
fn set_local_avatar() -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut profile = load_provider_profile()?
        .ok_or("secure VPR provider profile is not configured; run vpr-provider-credentials set")?;
    println!("Configure and select the self-hosted realtime avatar worker.");
    let endpoint = prompt_line("Local avatar HTTPS endpoint: ")?;
    let api_token = prompt_secret("Local avatar API token: ")?;
    profile.select_local_avatar(&endpoint, &api_token)?;
    probe_profile_local_avatar(&profile)?;
    save_provider_profile(&profile)?;
    println!("Selected local-open-source avatar securely for the current Windows user.");
    print_safe_profile(&profile);
    Ok(())
}

#[cfg(windows)]
fn use_did_avatar() -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut profile = load_provider_profile()?
        .ok_or("secure VPR provider profile is not configured; run vpr-provider-credentials set")?;
    profile.select_did_avatar();
    probe_profile_did(&profile)?;
    save_provider_profile(&profile)?;
    println!("Selected D-ID avatar for the current Windows user.");
    print_safe_profile(&profile);
    Ok(())
}

#[cfg(windows)]
fn import_env_profile() -> Result<(), Box<dyn Error + Send + Sync>> {
    require_canonical_provider_if_set("VPR_OWNER_LAB_STT_PROVIDER", "deepgram")?;
    require_canonical_provider_if_set("VPR_OWNER_LAB_LLM_PROVIDER", "deepseek")?;
    let deepgram_api_key = required_process_value("VPR_OWNER_LAB_STT_API_KEY")?;
    let deepseek_api_key = required_process_value("VPR_OWNER_LAB_LLM_API_KEY")?;
    let avatar_provider = avatar_provider_from_process_environment()?;
    let profile = match avatar_provider.as_str() {
        "did" => ProviderCredentialProfile::canonical_rt0(
            &required_process_value("VPR_DID_API_KEY")?,
            &required_process_value("VPR_DID_AGENT_ID")?,
            deepgram_api_key,
            deepseek_api_key,
        ),
        "local-open-source" => ProviderCredentialProfile::local_rt0(
            &required_process_value("VPR_LOCAL_AVATAR_ENDPOINT")?,
            &required_process_value("VPR_LOCAL_AVATAR_API_TOKEN")?,
            deepgram_api_key,
            deepseek_api_key,
        ),
        _ => unreachable!("avatar provider resolver returns canonical names only"),
    };
    probe_selected_avatar(&profile)?;
    save_provider_profile(&profile)?;
    println!("Imported current CMD provider credentials into Windows Credential Manager.");
    print_safe_profile(&profile);
    Ok(())
}

#[cfg(windows)]
fn avatar_provider_from_process_environment() -> Result<String, Box<dyn Error + Send + Sync>> {
    if let Some(explicit) = optional_process_value("VPR_OWNER_LAB_AVATAR_PROVIDER") {
        return match explicit.to_ascii_lowercase().as_str() {
            "did" | "d-id" | "did-agent-streams" => Ok("did".into()),
            "local" | "local-open-source" => Ok("local-open-source".into()),
            _ => Err("VPR_OWNER_LAB_AVATAR_PROVIDER selects an unsupported avatar provider".into()),
        };
    }
    let did_complete = optional_process_value("VPR_DID_API_KEY").is_some()
        && optional_process_value("VPR_DID_AGENT_ID").is_some();
    let local_complete = optional_process_value("VPR_LOCAL_AVATAR_ENDPOINT").is_some()
        && optional_process_value("VPR_LOCAL_AVATAR_API_TOKEN").is_some();
    match (did_complete, local_complete) {
        (true, false) => Ok("did".into()),
        (false, true) => Ok("local-open-source".into()),
        _ => Err(
            "set VPR_OWNER_LAB_AVATAR_PROVIDER when avatar configuration is missing or ambiguous"
                .into(),
        ),
    }
}

#[cfg(windows)]
fn optional_process_value(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
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
fn probe_avatar() -> Result<(), Box<dyn Error + Send + Sync>> {
    let profile = load_provider_profile()?
        .ok_or("secure VPR provider profile is not configured; run vpr-provider-credentials set")?;
    probe_selected_avatar(&profile)
}

#[cfg(windows)]
fn probe_selected_avatar(
    profile: &ProviderCredentialProfile,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    match profile.avatar_provider.as_str() {
        "did" | "d-id" | "did-agent-streams" => probe_profile_did(profile),
        "local" | "local-open-source" => probe_profile_local_avatar(profile),
        _ => Err("secure VPR provider profile selects an unsupported avatar provider".into()),
    }
}

#[cfg(windows)]
fn probe_profile_local_avatar(
    profile: &ProviderCredentialProfile,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let endpoint = profile
        .local_avatar_endpoint
        .as_deref()
        .ok_or("local avatar endpoint is not configured")?;
    let api_token = profile
        .local_avatar_api_token
        .as_deref()
        .ok_or("local avatar API token is not configured")?;
    let provider =
        LocalOpenSourceAvatar::new(LocalOpenSourceAvatarConfig::new(endpoint, api_token))
            .map_err(|_| "local avatar provider configuration rejected")?;
    provider.probe_health().map_err(|error| {
        let message = match error.kind {
            ProviderErrorKind::PolicyDenied => "local avatar probe: POLICY_DENIED",
            ProviderErrorKind::RateLimited => "local avatar probe: RATE_LIMITED",
            ProviderErrorKind::Timeout => "local avatar probe: TIMEOUT",
            ProviderErrorKind::Unavailable => "local avatar probe: UNAVAILABLE",
            ProviderErrorKind::Cancelled => "local avatar probe: CANCELLED",
            ProviderErrorKind::InsufficientCredits => "local avatar probe: INSUFFICIENT_CREDITS",
            ProviderErrorKind::InvalidResponse => "local avatar probe: INVALID_RESPONSE",
        };
        Box::<dyn Error + Send + Sync>::from(message)
    })?;
    println!("Local avatar worker probe: OK");
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

    match provider.probe_account_auth_detailed() {
        Ok(()) => println!("D-ID account authentication: OK"),
        Err(DidRuntimeAccessFailure::Unauthorized) => {
            return Err(
                "D-ID account authentication: UNAUTHORIZED_401 (D-ID rejected the API key itself)"
                    .into(),
            );
        }
        Err(DidRuntimeAccessFailure::Forbidden) => {
            return Err(
                "D-ID account authentication: FORBIDDEN_403 (the key reached D-ID but account-level API access was denied)"
                    .into(),
            );
        }
        Err(DidRuntimeAccessFailure::Provider(error)) => {
            let message = match error.kind {
                ProviderErrorKind::PolicyDenied => "D-ID account authentication: POLICY_DENIED",
                ProviderErrorKind::RateLimited => "D-ID account authentication: RATE_LIMITED",
                ProviderErrorKind::Timeout => "D-ID account authentication: TIMEOUT",
                ProviderErrorKind::Unavailable => "D-ID account authentication: UNAVAILABLE",
                ProviderErrorKind::Cancelled => "D-ID account authentication: CANCELLED",
                ProviderErrorKind::InsufficientCredits => {
                    "D-ID account authentication: INSUFFICIENT_CREDITS"
                }
                ProviderErrorKind::InvalidResponse => {
                    "D-ID account authentication: INVALID_RESPONSE"
                }
            };
            return Err(message.into());
        }
    }

    match provider.probe_runtime_access_detailed() {
        Ok(DidRuntimeAccessProbe::Presenter(presenter)) => {
            println!("D-ID credential probe: OK (presenter={presenter})");
            // A credential probe is not a realtime-session readiness probe.
            // Expressive V2 requires a private, server-owned Echo sender.
            // Do not let the launcher report READY then show a black avatar.
            if presenter.eq_ignore_ascii_case("expressive")
                && env::var("VPR_DID_ECHO_ENABLED").ok().as_deref() != Some("true")
            {
                return Err(
                    "RT0_EXPRESSIVE_ECHO_REQUIRED: D-ID credentials are valid, but this Expressive presenter cannot open a secure session without server-owned Echo. The current credential profile contains only D-ID/Deepgram/DeepSeek, not a voice provider. The launcher must not claim avatar readiness. Configure server-owned Echo with a voice service; do not disable the revocation guard."
                        .into(),
                );
            }
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
                ProviderErrorKind::InsufficientCredits => {
                    "D-ID credential probe: INSUFFICIENT_CREDITS"
                }
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
    match profile.avatar_provider.as_str() {
        "local" | "local-open-source" => println!(
            "  avatar: local-open-source ({}) · token stored",
            profile
                .local_avatar_endpoint
                .as_deref()
                .unwrap_or("<missing>")
        ),
        _ => println!(
            "  avatar: D-ID ({}) · agent={} · legacy fluent disabled",
            profile.did_endpoint,
            redact_identifier(&profile.did_agent_id)
        ),
    }
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
