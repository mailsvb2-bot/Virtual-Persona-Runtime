import os
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
    os.chdir(ROOT)
    os.execvpe(
        "cargo",
        ["cargo", "run", "--quiet", "-p", "vpr-owner-lab", "--bin", "vpr-owner-lab"],
        env,
    )


if __name__ == "__main__":
    main()
