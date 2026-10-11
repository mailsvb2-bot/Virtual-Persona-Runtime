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


def verify_system_voice(python: Path) -> None:
    """Verify real native speech and Echo-compatible PCM before replacing Owner Lab.

    Use the exact bridge synthesizer, not a registry-only voice lookup. The
    probe runs locally with no bearer token, network listener, or persisted WAV.
    """
    bridge_dir = Path(__file__).resolve().parent
    probe = (
        "import sys; sys.path.insert(0, sys.argv[1]); "
        "from windows_sapi_tts_bridge import make_windows_sapi_synthesizer; "
        "make_windows_sapi_synthesizer()('Проверка голоса.')"
    )
    try:
        result = subprocess.run(
            [str(python), "-c", probe, str(bridge_dir)],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            timeout=45, check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise RuntimeError(
            "Microsoft Irina Desktop preflight failed; existing Owner Lab was not stopped"
        ) from error
    if result.returncode != 0:
        raise RuntimeError(
            "Microsoft Irina Desktop (ru-RU, Female) is missing or cannot produce "
            "Echo-compatible PCM; existing Owner Lab was not stopped"
        )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--data-dir", type=Path, required=True)
    args = parser.parse_args()
    root = args.data_dir.expanduser().resolve()
    root.mkdir(parents=True, exist_ok=True)
    python = provision(root)
    verify_system_voice(python)
    print("ECHO_PYTHON=" + str(python))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Windows voice preparation failed: {error}", file=sys.stderr)
        raise SystemExit(1)
