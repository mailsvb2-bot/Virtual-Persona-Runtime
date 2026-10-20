import os
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ENV = {
    "VPR_DID_ENDPOINT": "http://127.0.0.1:18788",
    "VPR_DID_API_KEY": "backend-e2e-secret",
    "VPR_DID_AGENT_ID": "backend-e2e-agent",
    "VPR_OWNER_LAB_ALLOW_EGRESS": "true",
    "VPR_OWNER_LAB_PORT": "18787",
}


def main() -> None:
    env = os.environ.copy()
    env.update(ENV)
    # Hermetic per-launch persistence: the real backend must commit owner review.
    # mkdtemp creates a private directory; no fallback to fake success.
    env["VPR_OWNER_LAB_PERSONA_STORE_PATH"] = str(
        Path(tempfile.mkdtemp(prefix="vpr-owner-lab-e2e-")) / "reviewed-persona.json"
    )
    os.chdir(ROOT)
    os.execvpe(
        "cargo",
        ["cargo", "run", "--quiet", "-p", "vpr-owner-lab", "--bin", "vpr-owner-lab"],
        env,
    )


if __name__ == "__main__":
    main()
