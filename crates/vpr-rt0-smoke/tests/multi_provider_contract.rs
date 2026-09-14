mod support;

use std::io::Write;
use std::net::TcpListener;
use std::thread;

use vpr_policy::ConsentState;
use vpr_rt0_smoke::{SmokeConfig, SmokeProviderKind, run};

fn serve_once(body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        support::read_complete_http_request(&mut stream);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}"
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    format!("http://{address}/v1/provider-test")
}

fn run_provider(kind: SmokeProviderKind, body: &'static str) -> vpr_rt0_smoke::SmokeRun {
    run(SmokeConfig::new(
        serve_once(body),
        "secret-not-evidence",
        "test-model",
        "ru-RU",
        "Ответь кратко",
    )
    .with_provider_kind(kind)
    .with_provider_policy_allow(true)
    .with_consent(ConsentState::Granted))
    .unwrap()
}

#[test]
fn one_canonical_runtime_path_accepts_three_provider_protocols() {
    let openai = run_provider(
        SmokeProviderKind::OpenAiCompatible,
        concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Привет!\"}}],\"usage\":null}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        ),
    );
    let anthropic = run_provider(
        SmokeProviderKind::Anthropic,
        concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5,\"output_tokens\":0}}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Привет!\"}}\n\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":2}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        ),
    );
    let gemini = run_provider(
        SmokeProviderKind::Gemini,
        concat!(
            "data: {\"event_type\":\"step.delta\",\"delta\":{\"type\":\"text\",\"text\":\"Привет!\"}}\n\n",
            "data: {\"event_type\":\"interaction.completed\",\"interaction\":{\"status\":\"completed\",\"usage\":{\"total_input_tokens\":5,\"total_output_tokens\":2}}}\n\n",
            "data: [DONE]\n\n"
        ),
    );

    for result in [&openai, &anthropic, &gemini] {
        assert_eq!(result.response_text, "Привет!");
        assert_eq!(result.evidence.runtime_path, "authorized_llm_generation");
        assert_eq!(result.evidence.input_units, Some(5));
        assert_eq!(result.evidence.input_unit.as_deref(), Some("token"));
        assert_eq!(result.evidence.output_units, Some(2));
        assert_eq!(result.evidence.output_unit.as_deref(), Some("token"));
        assert!(!result.evidence.output_delivery_proven);
    }
    assert_eq!(openai.evidence.provider, "openai-compatible");
    assert_eq!(anthropic.evidence.provider, "anthropic");
    assert_eq!(gemini.evidence.provider, "gemini");
}
