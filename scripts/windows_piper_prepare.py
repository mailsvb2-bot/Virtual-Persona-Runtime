"""Idempotent, verified first-run provisioning for the optional Piper RT0 voice.

Only executed by explicit -Piper Windows launch. Never run on import.
Packages stay in the user's private data directory, not in the VPR checkout.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from urllib.request import Request, urlopen

VOICE = "ru_RU-denis-medium"
MODEL_SHA256 = "15fab56e11a097858ee115545d0f697fc2a316c41a291a5362349fb870411b0a"
VOICE_URL = (
    "https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/"
    "ru/ru_RU/denis/medium/"
)
MAX_MODEL_BYTES = 80 * 1024 * 1024
MAX_CONFIG_BYTES = 32 * 1024
PIPER_VERSION = "1.3.0"


def model_is_verified(path: Path) -> bool:
    if not path.is_file() or not 60_000_000 <= path.stat().st_size <= MAX_MODEL_BYTES:
        return False
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest() == MODEL_SHA256


def config_is_valid(path: Path) -> bool:
    try:
        if not path.is_file() or path.stat().st_size > MAX_CONFIG_BYTES:
            return False
        data = json.loads(path.read_text(encoding="utf-8"))
        return (
            data.get("language", {}).get("code") == "ru_RU"
            and data.get("audio", {}).get("sample_rate") == 22050
            and data.get("dataset") == "denis"
            and data.get("num_speakers") == 1
        )
    except (OSError, ValueError, TypeError, AttributeError):
        return False


def download_checked(url: str, destination: Path, limit: int, validator) -> None:
    """Atomic download from the pinned official voice revision, with fail-closed validation."""
    staging = destination.with_name(destination.name + ".partial")
    staging.unlink(missing_ok=True)
    try:
        req = Request(url, headers={"User-Agent": "VPR-RT0-Piper/1"})
        with urlopen(req, timeout=40) as response, staging.open("xb") as output:
            if response.url.split("/", 3)[:3] not in (
                ["https:", "", "huggingface.co"],
                ["https:", "", "cas-bridge.xethub.hf.co"],
                ["https:", "", "cdn-lfs.huggingface.co"],
            ):
                raise RuntimeError("untrusted model download host")
            total = 0
            while True:
                block = response.read(min(1024 * 1024, limit + 1 - total))
                if not block:
                    break
                total += len(block)
                if total > limit:
                    raise RuntimeError("model download exceeds size limit")
                output.write(block)
        if not validator(staging):
            raise RuntimeError("download integrity or voice identity validation failed")
        os.replace(staging, destination)
    finally:
        staging.unlink(missing_ok=True)


def provision_voice(data_dir: Path) -> Path:
    voice_dir = data_dir / "voices"
    voice_dir.mkdir(parents=True, exist_ok=True)
    model = voice_dir / (VOICE + ".onnx")
    config = voice_dir / (VOICE + ".onnx.json")
    if not model_is_verified(model):
        download_checked(VOICE_URL + model.name, model, MAX_MODEL_BYTES, model_is_verified)
    if not config_is_valid(config):
        download_checked(VOICE_URL + config.name, config, MAX_CONFIG_BYTES, config_is_valid)
    if not model_is_verified(model) or not config_is_valid(config):
        raise RuntimeError("Piper voice validation failed")
    return model


def provision_runtime(data_dir: Path) -> Path:
    """Keep the optional GPL engine isolated from the existing Echo worker."""
    venv = data_dir / "venv"
    python = venv / "Scripts" / "python.exe" if os.name == "nt" else venv / "bin" / "python"
    if not python.is_file():
        subprocess.run([sys.executable, "-m", "venv", str(venv)], check=True)
    installed = subprocess.run(
        [str(python), "-c", "import importlib.metadata as m; import piper; "
         "assert m.version('piper-tts') == '1.3.0'"],
        check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    ).returncode == 0
    if not installed:
        subprocess.run([
            str(python), "-m", "pip", "install", "--disable-pip-version-check",
            "--only-binary=:all:", "piper-tts==" + PIPER_VERSION,
        ], check=True)
        subprocess.run([
            str(python), "-c",
            "import importlib.metadata as m; import piper; "
            "assert m.version('piper-tts') == '1.3.0'",
        ], check=True)
    return python


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--data-dir", type=Path, required=True)
    args = parser.parse_args()
    root = args.data_dir.expanduser().resolve()
    root.mkdir(parents=True, exist_ok=True)
    # Download before attempting to start a voice service; no silent fallback.
    python = provision_runtime(root)
    model = provision_voice(root)
    print("PIPER_PYTHON=" + str(python))
    print("PIPER_MODEL=" + str(model))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Piper preparation failed: {error}", file=sys.stderr)
        raise SystemExit(1)
