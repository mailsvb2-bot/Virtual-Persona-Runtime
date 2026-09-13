use std::error::Error;
use std::fmt::{Display, Formatter};
use std::time::Instant;

use serde::Serialize;
use vpr_domain::{
    CorrelationId, PersonaId, PersonaIdentity, PersonaMode, PersonaVersion, Rt0ReasonCode,
    SessionId, TurnId,
};
use vpr_integration::{
    GeneratedTextBuffer, LlmPort, LlmRequest, ProviderError, UsageEvidence, UsageUnit,
};
use vpr_policy::{AuthorityLayer, AuthorityScope, ConsentState, EffectiveAuthority};
use vpr_provider_anthropic::{AnthropicConfig, AnthropicLlm};
use vpr_provider_gemini::{GeminiConfig, GeminiLlm};
use vpr_provider_openai_compatible::{OpenAiCompatibleConfig, OpenAiCompatibleLlm};
use vpr_runtime::{ActiveSession, ActiveTurn, ProviderExecutionError, SessionSecurityConfig};

const PROVIDER_SCOPE: &str = "provider.egress";
const PERSONA_ID: &str = "rt0-smoke-persona";
const SESSION_ID: &str = "rt0-smoke-session";
const TURN_ID: &str = "rt0-smoke-turn";
const CORRELATION_ID: &str = "rt0-smoke-correlation";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmokeProviderKind {
    OpenAiCompatible,
    Anthropic,
    Gemini,
}

pub struct SmokeConfig {
    provider_kind: SmokeProviderKind,
    endpoint: String,
    api_key: String,
    model: String,
    locale: String,
    prompt: String,
    provider_policy_allows: bool,
    consent: ConsentState,
}
impl SmokeConfig {
    #[must_use]
    pub fn new(
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
        locale: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        Self {
            provider_kind: SmokeProviderKind::OpenAiCompatible,
            endpoint: endpoint.into(),
            api_key: api_key.into(),
            model: model.into(),
            locale: locale.into(),
            prompt: prompt.into(),
            provider_policy_allows: false,
            consent: ConsentState::Missing,
        }
    }

    #[must_use]
    pub const fn with_provider_kind(mut self, provider_kind: SmokeProviderKind) -> Self {
        self.provider_kind = provider_kind;
        self
    }

    #[must_use]
    pub const fn with_provider_policy_allow(mut self, allows: bool) -> Self {
        self.provider_policy_allows = allows;
        self
    }

    #[must_use]
    pub const fn with_consent(mut self, consent: ConsentState) -> Self {
        self.consent = consent;
        self
    }
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SmokeEvidence {
    pub runtime_path: String,
    pub provider: String,
    pub model: String,
    pub latency_millis: u64,
    pub input_units: Option<u64>,
    pub input_unit: Option<String>,
    pub output_units: Option<u64>,
    pub output_unit: Option<String>,
    pub estimated_cost_microunits: Option<u64>,
    pub provider_charge_microunits: Option<u64>,
    pub output_chars: u64,
    pub provider_policy_allows: bool,
    pub consent_granted: bool,
    pub output_delivery_proven: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmokeRun {
    pub response_text: String,
    pub evidence: SmokeEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmokeError {
    Runtime(Rt0ReasonCode),
    ProviderConfiguration(ProviderError),
    ProviderExecution(ProviderExecutionError),
}
impl Display for SmokeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Runtime(reason) => write!(formatter, "runtime denied smoke path: {reason:?}"),
            Self::ProviderConfiguration(error) => {
                write!(formatter, "provider configuration failed: {:?}", error.kind)
            }
            Self::ProviderExecution(error) => write!(
                formatter,
                "provider execution failed: {:?}",
                error.reason_code()
            ),
        }
    }
}

impl Error for SmokeError {}

/// Runs one authorized RT0 LLM generation path and returns non-secret evidence.
///
/// The generated sink is intentionally not a transport. This smoke proves provider generation
/// through current runtime authority/egress checks; user-visible delivery remains a separate gate.
///
/// # Errors
/// Returns a typed runtime/provider error when any fail-closed gate or provider operation fails.
pub fn run(config: SmokeConfig) -> Result<SmokeRun, SmokeError> {
    let SmokeConfig {
        provider_kind,
        endpoint,
        api_key,
        model,
        locale,
        prompt,
        provider_policy_allows,
        consent,
    } = config;
    let persona = smoke_persona()?;
    let authority = provider_authority()?;
    let mut session = ActiveSession::new(
        SessionId::new(SESSION_ID)
            .map_err(|_| SmokeError::Runtime(Rt0ReasonCode::InternalError))?,
        persona.id().clone(),
        SessionSecurityConfig::new(authority, None, provider_policy_allows, consent, false),
    );
    session.activate().map_err(SmokeError::Runtime)?;

    let turn = ActiveTurn::new(
        TurnId::new(TURN_ID).map_err(|_| SmokeError::Runtime(Rt0ReasonCode::InternalError))?,
        CorrelationId::new(CORRELATION_ID)
            .map_err(|_| SmokeError::Runtime(Rt0ReasonCode::InternalError))?,
        &persona,
        &session,
    )
    .map_err(|reason| SmokeError::Runtime(reason.reason_code()))?;
    turn.authorize().map_err(SmokeError::Runtime)?;
    turn.begin_processing().map_err(SmokeError::Runtime)?;

    let provider = build_provider(provider_kind, endpoint, api_key, model)?;
    let descriptor = provider.descriptor();
    let request = LlmRequest {
        locale,
        context: prompt,
    };
    let mut generated = GeneratedTextBuffer::default();
    let started = Instant::now();
    let usage = turn
        .execute_llm(provider.as_ref(), &request, &mut generated)
        .map_err(SmokeError::ProviderExecution)?;
    let latency_millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let output_chars = u64::try_from(generated.as_str().chars().count()).unwrap_or(u64::MAX);

    Ok(SmokeRun {
        response_text: generated.into_string(),
        evidence: smoke_evidence(
            descriptor.provider,
            descriptor.model,
            latency_millis,
            output_chars,
            &usage,
            provider_policy_allows,
            consent,
        ),
    })
}
fn build_provider(
    provider_kind: SmokeProviderKind,
    endpoint: String,
    api_key: String,
    model: String,
) -> Result<Box<dyn LlmPort>, SmokeError> {
    let provider: Box<dyn LlmPort> = match provider_kind {
        SmokeProviderKind::OpenAiCompatible => Box::new(
            OpenAiCompatibleLlm::new(OpenAiCompatibleConfig::new(endpoint, api_key, model))
                .map_err(SmokeError::ProviderConfiguration)?,
        ),
        SmokeProviderKind::Anthropic => Box::new(
            AnthropicLlm::new(AnthropicConfig::new(endpoint, api_key, model))
                .map_err(SmokeError::ProviderConfiguration)?,
        ),
        SmokeProviderKind::Gemini => Box::new(
            GeminiLlm::new(GeminiConfig::new(endpoint, api_key, model))
                .map_err(SmokeError::ProviderConfiguration)?,
        ),
    };
    Ok(provider)
}

fn smoke_persona() -> Result<PersonaIdentity, SmokeError> {
    let id = PersonaId::new(PERSONA_ID)
        .map_err(|_| SmokeError::Runtime(Rt0ReasonCode::InternalError))?;
    let version =
        PersonaVersion::new(1).ok_or(SmokeError::Runtime(Rt0ReasonCode::InternalError))?;
    Ok(PersonaIdentity::new(id, version, PersonaMode::DigitalTwin))
}

fn provider_authority() -> Result<EffectiveAuthority, SmokeError> {
    let scope = AuthorityScope::new(PROVIDER_SCOPE)
        .ok_or(SmokeError::Runtime(Rt0ReasonCode::InternalError))?;
    Ok(EffectiveAuthority::compose(&[AuthorityLayer::new(
        [scope],
        [],
    )]))
}

const fn usage_unit_name(unit: UsageUnit) -> &'static str {
    match unit {
        UsageUnit::Token => "token",
        UsageUnit::TextCharacter => "text_character",
        UsageUnit::AudioMillisecond => "audio_millisecond",
    }
}

fn smoke_evidence(
    provider: String,
    model: String,
    latency_millis: u64,
    output_chars: u64,
    usage: &UsageEvidence,
    provider_policy_allows: bool,
    consent: ConsentState,
) -> SmokeEvidence {
    SmokeEvidence {
        runtime_path: "authorized_llm_generation".into(),
        provider,
        model,
        latency_millis,
        input_units: usage.input_units,
        input_unit: usage.input_unit.map(usage_unit_name).map(str::to_owned),
        output_units: usage.output_units,
        output_unit: usage.output_unit.map(usage_unit_name).map(str::to_owned),
        estimated_cost_microunits: usage.estimated_cost_microunits,
        provider_charge_microunits: usage.provider_charge_microunits,
        output_chars,
        provider_policy_allows,
        consent_granted: consent == ConsentState::Granted,
        output_delivery_proven: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn serve_once(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 8192];
            let _ = stream.read(&mut request).unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}"
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        format!("http://{address}/v1/chat/completions")
    }
    #[test]
    fn authorized_smoke_uses_runtime_and_real_adapter_contract() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Привет\"}}],\"usage\":null}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"!\"}}],\"usage\":null}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        );
        let config = SmokeConfig::new(
            serve_once(body),
            "secret",
            "test-model",
            "ru-RU",
            "Ответь кратко",
        )
        .with_provider_policy_allow(true)
        .with_consent(ConsentState::Granted);
        let result = run(config).unwrap();

        assert_eq!(result.response_text, "Привет!");
        assert_eq!(result.evidence.input_units, Some(5));
        assert_eq!(result.evidence.input_unit.as_deref(), Some("token"));
        assert_eq!(result.evidence.output_units, Some(2));
        assert_eq!(result.evidence.output_unit.as_deref(), Some("token"));
        assert_eq!(result.evidence.output_chars, 7);
        assert!(!result.evidence.output_delivery_proven);
    }
    #[test]
    fn smoke_defaults_fail_closed_before_network_egress() {
        let config = SmokeConfig::new(
            "http://127.0.0.1:1/v1/chat/completions",
            "secret",
            "test-model",
            "ru-RU",
            "test",
        );
        let error = run(config).unwrap_err();
        assert!(matches!(
            error,
            SmokeError::ProviderExecution(ProviderExecutionError::Denied(_))
        ));
        assert_eq!(
            match error {
                SmokeError::ProviderExecution(inner) => inner.reason_code(),
                _ => unreachable!(),
            },
            Rt0ReasonCode::EgressDenied
        );
    }

    #[test]
    fn explicit_provider_allow_still_requires_consent() {
        let config = SmokeConfig::new(
            "http://127.0.0.1:1/v1/chat/completions",
            "secret",
            "test-model",
            "ru-RU",
            "test",
        )
        .with_provider_policy_allow(true);
        let error = run(config).unwrap_err();
        assert_eq!(
            match error {
                SmokeError::ProviderExecution(inner) => inner.reason_code(),
                _ => unreachable!(),
            },
            Rt0ReasonCode::ConsentRequired
        );
    }
}
