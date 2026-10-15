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
from urllib.parse import urlsplit
from urllib.request import Request, urlopen

TRUSTED_VOICE_HOSTS = frozenset({
    "huggingface.co",
    "cas-bridge.xethub.hf.co",
    "cdn-lfs.huggingface.co",
})


def is_trusted_voice_redirect(url: str) -> bool:
    """Allow only pinned-model CDN transports over authenticated HTTPS origin."""
    try:
        parsed = urlsplit(url)
        host = (parsed.hostname or "").lower()
        return (
            parsed.scheme == "https"
            and parsed.username is None
            and parsed.password is None
            and parsed.port in (None, 443)
            and (host in TRUSTED_VOICE_HOSTS or host.endswith(".cdn.hf.co"))
        )
    except ValueError:
        return False

# Pinned upstream v1.0.0 LFS SHA-256; Irina is dev/interim only
# (RHVoice dataset rights unknown); Denis dataset CC0.
VOICES = {
    "ru_RU-irina-medium": ("irina", "8ff38212d23da300bbe3705c645e6e5b9475f0bfde01558eb17813e22acaaaaa"),
    "ru_RU-denis-medium": ("denis", "15fab56e11a097858ee115545d0f697fc2a316c41a291a5362349fb870411b0a"),
}
DEFAULT_VOICE = "ru_RU-irina-medium"
VOICE = DEFAULT_VOICE
MODEL_SHA256 = VOICES[DEFAULT_VOICE][1]
VOICE_BASE_URL = "https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/ru/ru_RU/"


def voice_url(voice: str = DEFAULT_VOICE) -> str:
    return f"{VOICE_BASE_URL}{VOICES[voice][0]}/medium/"


VOICE_URL = voice_url()
MAX_MODEL_BYTES = 80 * 1024 * 1024
MAX_CONFIG_BYTES = 32 * 1024
PIPER_VERSION = "1.3.0"
ECHO_LIVEKIT_VERSION = "1.1.19"


def model_is_verified(path: Path, voice: str = DEFAULT_VOICE) -> bool:
    if not path.is_file() or not 60_000_000 <= path.stat().st_size <= MAX_MODEL_BYTES:
        return False
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest() == VOICES[voice][1]


def config_is_valid(path: Path, voice: str = DEFAULT_VOICE) -> bool:
    try:
        if not path.is_file() or path.stat().st_size > MAX_CONFIG_BYTES:
            return False
        data = json.loads(path.read_text(encoding="utf-8"))
        return (
            data.get("language", {}).get("code") == "ru_RU"
            and data.get("audio", {}).get("sample_rate") == 22050
            and data.get("dataset") == VOICES[voice][0]
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
            if not is_trusted_voice_redirect(response.url):
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


def provision_voice(data_dir: Path, voice: str = DEFAULT_VOICE) -> Path:
    if voice not in VOICES:
        raise ValueError("unsupported Piper voice")
    voice_dir = data_dir / "voices"
    voice_dir.mkdir(parents=True, exist_ok=True)
    model = voice_dir / (voice + ".onnx")
    config = voice_dir / (voice + ".onnx.json")
    base = voice_url(voice)
    def model_ok(path):
        return model_is_verified(path, voice)
    def config_ok(path):
        return config_is_valid(path, voice)
    if not model_ok(model):
        download_checked(base + model.name, model, MAX_MODEL_BYTES, model_ok)
    if not config_ok(config):
        download_checked(base + config.name, config, MAX_CONFIG_BYTES, config_ok)
    if not model_ok(model) or not config_ok(config):
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
    # Private D-ID Echo runs as a separate worker process, but it needs the
    # real LiveKit RTC SDK. Keep it in this isolated local runtime and verify
    # the exact PCM byte-stream API before allowing an RT0 launch.
    echo_probe = (
        "import importlib.metadata as m; from livekit import rtc; "
        f"assert m.version('livekit') == '{ECHO_LIVEKIT_VERSION}'; "
        "assert hasattr(rtc.LocalParticipant, 'stream_bytes')"
    )
    echo_ready = subprocess.run(
        [str(python), "-c", echo_probe],
        check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    ).returncode == 0
    if not echo_ready:
        subprocess.run([
            str(python), "-m", "pip", "install", "--disable-pip-version-check",
            "--only-binary=:all:", "livekit==" + ECHO_LIVEKIT_VERSION,
        ], check=True)
        subprocess.run([str(python), "-c", echo_probe], check=True)
    return python


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--data-dir", type=Path, required=True)
    parser.add_argument("--voice", choices=sorted(VOICES), default=DEFAULT_VOICE)
    args = parser.parse_args()
    root = args.data_dir.expanduser().resolve()
    root.mkdir(parents=True, exist_ok=True)
    # Download before attempting to start a voice service; no silent fallback.
    python = provision_runtime(root)
    model = provision_voice(root, args.voice)
    print("PIPER_PYTHON=" + str(python))
    print("PIPER_MODEL=" + str(model))
    print("PIPER_VOICE=" + args.voice)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Piper preparation failed: {error}", file=sys.stderr)
        raise SystemExit(1)
