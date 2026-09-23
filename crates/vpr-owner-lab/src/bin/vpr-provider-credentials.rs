use std::env;
use std::error::Error;

#[cfg(windows)]
use std::io::{self, Write};
#[cfg(windows)]
use vpr_owner_lab::{
    ProviderCredentialProfile, delete_provider_profile, load_provider_profile,
    save_provider_profile,
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
        "import-env" => import_env_profile(),
        "status" => status(),
        "clear" => clear(),
        _ => Err("usage: vpr-provider-credentials <set|import-env|status|clear>".into()),
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
        did_api_key,
        did_agent_id,
        deepgram_api_key,
        deepseek_api_key,
    );
    save_provider_profile(&profile)?;
    println!("Saved securely for the current Windows user.");
    print_safe_profile(&profile);
    Ok(())
}

#[cfg(windows)]
fn import_env_profile() -> Result<(), Box<dyn Error + Send + Sync>> {
    require_canonical_provider_if_set("VPR_OWNER_LAB_STT_PROVIDER", "deepgram")?;
    require_canonical_provider_if_set("VPR_OWNER_LAB_LLM_PROVIDER", "deepseek")?;
    let profile = ProviderCredentialProfile::canonical_rt0(
        required_process_value("VPR_DID_API_KEY")?,
        required_process_value("VPR_DID_AGENT_ID")?,
        required_process_value("VPR_OWNER_LAB_STT_API_KEY")?,
        required_process_value("VPR_OWNER_LAB_LLM_API_KEY")?,
    );
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
        "  avatar: D-ID ({}) · legacy fluent disabled",
        profile.did_endpoint
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
