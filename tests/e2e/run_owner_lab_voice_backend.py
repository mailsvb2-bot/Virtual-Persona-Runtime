import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ENV = {
    "VPR_DID_ENDPOINT": "http://127.0.0.1:18790",
    "VPR_DID_API_KEY": "voice-avatar-e2e-secret",
    "VPR_DID_AGENT_ID": "voice-e2e-agent",
    "VPR_OWNER_LAB_ALLOW_EGRESS": "true",
    "VPR_OWNER_LAB_PORT": "18789",
    "VPR_OWNER_LAB_STT_PROVIDER": "openai-transcription",
    "VPR_OWNER_LAB_STT_ENDPOINT": "http://127.0.0.1:18790/v1/audio/transcriptions",
    "VPR_OWNER_LAB_STT_API_KEY": "voice-stt-e2e-secret",
    "VPR_OWNER_LAB_STT_MODEL": "voice-stt-e2e",
    "VPR_OWNER_LAB_LLM_PROVIDER": "openai-compatible",
    "VPR_OWNER_LAB_LLM_ENDPOINT": "http://127.0.0.1:18790/v1/chat/completions",
    "VPR_OWNER_LAB_LLM_API_KEY": "voice-llm-e2e-secret",
    "VPR_OWNER_LAB_LLM_MODEL": "voice-llm-e2e",
}


def main() -> None:
    env = os.environ.copy()
    env.update(ENV)
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
