"""Hermetic real RTP sender for the RT1 local browser e2e."""
import os
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

def main():
    env = os.environ.copy()
    env.update({
        "RT1_WORKER_DB": str(Path(tempfile.mkdtemp(prefix="rt1-worker-canary-")) / "ledger.sqlite"),
        "RT1_WORKER_TOKEN": "rt1-local-e2e-secret",
        "RT1_WORKER_PORT": "18789",
    })
    os.chdir(ROOT)
    os.execvpe(sys.executable, [sys.executable, str(ROOT / "scripts/rt1_media_worker_server.py")], env)

if __name__ == "__main__":
    main()
