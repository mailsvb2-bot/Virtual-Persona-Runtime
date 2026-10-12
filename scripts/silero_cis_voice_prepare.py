"""Prepare the optional, MIT-licensed Silero CIS Base female-voice audition.

Installs only into its own user-local venv, never into Owner Lab's existing
Piper/SAPI runtimes. Network is used only for the fixed model and CPU torch.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import os
import subprocess
import sys

from silero_cis_voice_preview import (
    MODEL_NAME, MODEL_URL, MAX_MODEL_BYTES, verified_model,
)
from windows_piper_prepare import download_checked

TORCH_VERSION = "2.5.1"
NUMPY_VERSION = "1.26.4"
CPU_INDEX = "https://download.pytorch.org/whl/cpu"


def prepare_model(folder: Path) -> Path:
    folder.mkdir(parents=True, exist_ok=True)
    path = folder / MODEL_NAME
    if not verified_model(path):
        download_checked(MODEL_URL, path, MAX_MODEL_BYTES, verified_model)
    if not verified_model(path):
        raise RuntimeError("Untrusted Silero CIS model, refusing to load")
    return path


def prepare_python(root: Path) -> Path:
    venv = root / "venv"
    exe = venv / "Scripts" / "python.exe" if os.name == "nt" else venv / "bin" / "python"
    if not exe.is_file():
        subprocess.run([sys.executable, "-m", "venv", str(venv)], check=True)
    probe = [
        str(exe), "-c",
        "import torch; import numpy; import importlib.metadata as m; "
        f"assert m.version('torch').split('+')[0] == '{TORCH_VERSION}'; "
        f"assert m.version('numpy') == '{NUMPY_VERSION}'; "
        "assert torch.cuda.is_available() is False",
    ]
    if subprocess.run(
        probe, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False
    ).returncode != 0:
        subprocess.run([
            str(exe), "-m", "pip", "install", "--disable-pip-version-check",
            "--only-binary=:all:", "--index-url", CPU_INDEX,
            "--extra-index-url", "https://pypi.org/simple",
            "torch==" + TORCH_VERSION, "numpy==" + NUMPY_VERSION,
        ], check=True)
        subprocess.run(probe, check=True)
    return exe


def main() -> int:
    cli = argparse.ArgumentParser()
    cli.add_argument("--data-dir", type=Path, required=True)
    cli.add_argument("--model-only", action="store_true")
    args = cli.parse_args()
    root = args.data_dir.expanduser().resolve()
    model = prepare_model(root / "models")
    if args.model_only:
        print("VERIFIED_SILERO_MODEL=" + str(model))
        return 0
    python = prepare_python(root)
    print("VERIFIED_SILERO_PYTHON=" + str(python))
    print("VERIFIED_SILERO_MODEL=" + str(model))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print("Silero audition preparation failed: " + str(error), file=sys.stderr)
        raise SystemExit(1)
