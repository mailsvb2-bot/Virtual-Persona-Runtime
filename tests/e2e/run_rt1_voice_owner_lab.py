#!/usr/bin/env python3
"""Run the real Owner Lab HTTP application against loopback RT1 fixtures only."""
import os
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ENV = {
    "VPR_DID_ENDPOINT": "http://127.0.0.1:18790",
    "VPR_DID_API_KEY": "voice-avatar-e2e-secret",
    "VPR_DID_AGENT_ID": "voice-e2e-agent",
    "VPR_OWNER_LAB_ALLOW_EGRESS": "true",
    "VPR_OWNER_LAB_PORT": "18787",
    "VPR_OWNER_LAB_STT_PROVIDER": "openai-transcription",
    "VPR_OWNER_LAB_STT_ENDPOINT": "http://127.0.0.1:18790/v1/audio/transcriptions",
    "VPR_OWNER_LAB_STT_API_KEY": "voice-stt-e2e-secret",
    "VPR_OWNER_LAB_STT_MODEL": "voice-stt-e2e",
    "VPR_OWNER_LAB_LLM_PROVIDER": "openai-compatible",
    "VPR_OWNER_LAB_LLM_ENDPOINT": "http://127.0.0.1:18790/v1/chat/completions",
    "VPR_OWNER_LAB_LLM_API_KEY": "expressive-llm-e2e-secret",
    "VPR_OWNER_LAB_LLM_MODEL": "voice-llm-e2e",
    "VPR_OWNER_LAB_TTS_PROVIDER": "openai-speech",
    "VPR_OWNER_LAB_TTS_ENDPOINT": "http://127.0.0.1:18790/v1/audio/speech",
    "VPR_OWNER_LAB_TTS_API_KEY": "expressive-tts-e2e-secret",
    "VPR_OWNER_LAB_TTS_MODEL": "voice-e2e-model",
    "VPR_OWNER_LAB_TTS_VOICE": "alloy",
    "VPR_RT1_LOCAL_EXPERIMENT": "true",
    "VPR_RT1_DIAGNOSTIC_RTP": "false",
    "VPR_RT1_CANONICAL_VOICE_WORKER": "true",
    "RT1_WORKER_ENDPOINT": "http://127.0.0.1:18789",
    "RT1_WORKER_TOKEN": "rt1-local-e2e-secret",
}


def main() -> None:
    env = os.environ.copy()
    env.update(ENV)
    env["VPR_OWNER_LAB_PERSONA_STORE_PATH"] = str(
        Path(tempfile.mkdtemp(prefix="vpr-rt1-voice-e2e-")) / "reviewed-persona.json"
    )
    os.chdir(ROOT)
    os.execvpe(
        "cargo",
        [
            "cargo",
            "run",
            "--locked",
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
