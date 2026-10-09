import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ENV = {
    "VPR_DID_ENDPOINT": "http://127.0.0.1:18790",
    "VPR_DID_API_KEY": "expressive-avatar-e2e-secret",
    "VPR_DID_AGENT_ID": "voice-e2e-expressive-agent",
    # Expressive R0 is supported only by a private server-side Echo sender.
    # This harness never grants did.speak to the browser.
    "VPR_DID_ECHO_ENABLED": "true",
    "VPR_DID_ECHO_PYTHON": "python3",
    "VPR_DID_ECHO_TTS_ENDPOINT": "http://127.0.0.1:18790/v1/audio/speech",
    "VPR_DID_ECHO_TTS_API_KEY": "expressive-tts-e2e-secret",
    "VPR_DID_ECHO_TTS_MODEL": "fixture-tts",
    "VPR_DID_ECHO_TTS_VOICE": "fixture-voice",
    "VPR_OWNER_LAB_ALLOW_EGRESS": "true",
    "VPR_OWNER_LAB_PORT": "18791",
    "VPR_OWNER_LAB_STT_PROVIDER": "deepgram",
    "VPR_OWNER_LAB_STT_ENDPOINT": "http://127.0.0.1:18790/v1/listen",
    "VPR_OWNER_LAB_STT_API_KEY": "expressive-stt-e2e-secret",
    "VPR_OWNER_LAB_STT_MODEL": "nova-3",
    "VPR_OWNER_LAB_LLM_PROVIDER": "deepseek",
    "VPR_OWNER_LAB_LLM_ENDPOINT": "http://127.0.0.1:18790/v1/chat/completions",
    "VPR_OWNER_LAB_LLM_API_KEY": "expressive-llm-e2e-secret",
    "VPR_OWNER_LAB_LLM_MODEL": "deepseek-flash",
}


def main() -> None:
    env = os.environ.copy()
    env.update(ENV)
    # Only this hermetic fixture loads the local LiveKit stand-in; production
    # private Echo uses the installed LiveKit SDK and real provider transport.
    env["PYTHONPATH"] = str(ROOT / "tests/e2e/echo_fixture_site")
    os.chdir(ROOT)
    os.execvpe(
        "cargo",
        [
            "cargo",
            "run",
            "--quiet",
            "-p",
            "vpr-owner-lab",
            "--bin",
            "vpr-owner-lab",
        ],
        env,
    )


if __name__ == "__main__":
    main()