use std::env;
use std::error::Error;
use std::io::{Error as IoError, ErrorKind};

use vpr_policy::ConsentState;
use vpr_rt0_smoke::{SmokeConfig, SmokeProviderKind, run};

fn main() {
    if let Err(error) = run_cli() {
        eprintln!("RT0 LLM smoke failed: {error}");
        std::process::exit(1);
    }
}

fn run_cli() -> Result<(), Box<dyn Error>> {
    let provider_kind = match env::var("VPR_LLM_PROVIDER")
        .unwrap_or_else(|_| "openai-compatible".into())
        .as_str()
    {
        "openai-compatible" => SmokeProviderKind::OpenAiCompatible,
        "anthropic" => SmokeProviderKind::Anthropic,
        "gemini" => SmokeProviderKind::Gemini,
        value => {
            return Err(IoError::new(
                ErrorKind::InvalidInput,
                format!("unsupported VPR_LLM_PROVIDER: {value}"),
            )
            .into());
        }
    };
    let endpoint = required("VPR_LLM_ENDPOINT")?;
    let api_key = required("VPR_LLM_API_KEY")?;
    let model = required("VPR_LLM_MODEL")?;
    let locale = env::var("VPR_LLM_LOCALE").unwrap_or_else(|_| "ru-RU".into());
    let prompt = env::var("VPR_LLM_PROMPT")
        .unwrap_or_else(|_| "Ответь одним коротким предложением на русском языке.".into());
    let allow_egress = env::var("VPR_LLM_ALLOW_EGRESS").is_ok_and(|value| value == "true");
    let consent = if env::var("VPR_LLM_CONSENT").is_ok_and(|value| value == "granted") {
        ConsentState::Granted
    } else {
        ConsentState::Missing
    };
    let config = SmokeConfig::new(endpoint, api_key, model, locale, prompt)
        .with_provider_kind(provider_kind)
        .with_provider_policy_allow(allow_egress)
        .with_consent(consent);
    let result = run(config)?;

    println!("response:\n{}", result.response_text);
    println!(
        "evidence:\n{}",
        serde_json::to_string_pretty(&result.evidence)?
    );
    Ok(())
}

fn required(name: &'static str) -> Result<String, IoError> {
    env::var(name).map_err(|_| {
        IoError::new(
            ErrorKind::InvalidInput,
            format!("required environment variable {name} is not set"),
        )
    })
}
