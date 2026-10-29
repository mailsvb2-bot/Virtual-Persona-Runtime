"""Launch owner lab with a private persistent Persona and local RT1 canary."""
import os
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

def main():
    env = os.environ.copy()
    env.update({
        "VPR_DID_ENDPOINT": "http://127.0.0.1:18788",
        "VPR_DID_API_KEY": "backend-e2e-secret",
        "VPR_DID_AGENT_ID": "backend-e2e-agent",
        "VPR_OWNER_LAB_ALLOW_EGRESS": "true",
        "VPR_OWNER_LAB_PORT": "18787",
        "VPR_RT1_LOCAL_EXPERIMENT": "true",
        "VPR_RT1_DIAGNOSTIC_RTP": "true",
        "RT1_WORKER_ENDPOINT": "http://127.0.0.1:18789",
        "RT1_WORKER_TOKEN": "rt1-local-e2e-secret",
        "VPR_OWNER_LAB_PERSONA_STORE_PATH": str(
            Path(tempfile.mkdtemp(prefix="rt1-owner-canary-")) / "persona.json"
        ),
    })
    os.chdir(ROOT)
    os.execvpe(
        "cargo",
        ["cargo", "run", "--quiet", "-p", "vpr-owner-lab", "--bin", "vpr-owner-lab"],
        env,
    )

if __name__ == "__main__":
    main()
