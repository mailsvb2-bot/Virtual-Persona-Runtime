"""Install only the private Echo LiveKit SDK for Windows built-in Russian voice.

No voice models, web TTS or redundant Python runtimes are downloaded.
"""
import argparse
import os
from pathlib import Path
import subprocess
import sys

LIVEKIT_VERSION = "1.1.19"


def provision(data_dir: Path) -> Path:
    venv = data_dir / "venv"
    python = venv / "Scripts" / "python.exe" if os.name == "nt" else venv / "bin" / "python"
    if not python.is_file():
        subprocess.run([sys.executable, "-m", "venv", str(venv)], check=True)
    probe = ("import importlib.metadata as m; from livekit import rtc; "
             f"assert m.version('livekit') == '{LIVEKIT_VERSION}'; "
             "assert hasattr(rtc.LocalParticipant,'stream_bytes')")
    if subprocess.run(
        [str(python), "-c", probe],
        check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    ).returncode != 0:
        subprocess.run(
            [str(python), "-m", "pip", "install", "--disable-pip-version-check",
             "--only-binary=:all:", f"livekit=={LIVEKIT_VERSION}"],
            check=True,
        )
        subprocess.run([str(python), "-c", probe], check=True)
    return python


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--data-dir", type=Path, required=True)
    args = parser.parse_args()
    root = args.data_dir.expanduser().resolve()
    root.mkdir(parents=True, exist_ok=True)
    python = provision(root)
    print("ECHO_PYTHON=" + str(python))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Windows voice preparation failed: {error}", file=sys.stderr)
        raise SystemExit(1)
