"""RT1 GPU runtime dependency audit without importing GPU libraries.

The tested upstream MuseTalk revision prescribes Python 3.10 + torch 2.0.1
(CUDA 11.8 PyTorch wheel index) and the versions from requirements.txt.
This audit does not grant GPU or production readiness.
"""
from __future__ import annotations

import argparse
from importlib import metadata
import json
import sys

PYTHON = (3, 10)
REQUIRED = {
    "torch": "2.0.1",
    "torchvision": "0.15.2",
    "torchaudio": "2.0.2",
    "diffusers": "0.30.2",
    "accelerate": "0.28.0",
    "numpy": "1.23.5",
    "tensorflow": "2.12.0",
    "tensorboard": "2.12.0",
    "opencv-python": "4.9.0.80",
    "soundfile": "0.12.1",
    "transformers": "4.39.2",
    "huggingface-hub": "0.30.2",
    "librosa": "0.11.0",
    "einops": "0.8.1",
    "gradio": "5.24.0",
    "mmcv": "2.0.1",
    "mmdet": "3.1.0",
    "mmpose": "1.1.0",
}


def audit(*, python_version=None, installed=None):
    """Report all dependency mismatches, never attempt installs/downloads."""
    python_version = python_version or sys.version_info[:2]
    installed = metadata.version if installed is None else installed
    issues = []
    if tuple(python_version) != PYTHON:
        issues.append(f"Python {PYTHON[0]}.{PYTHON[1]} required by pinned upstream recipe")
    present = {}
    for package, expected in REQUIRED.items():
        try:
            actual = installed(package)
        except metadata.PackageNotFoundError:
            actual = None
        present[package] = actual
        # CUDA local wheel suffix e.g. torch 2.0.1+cu118 is allowed.
        if actual is None or actual.split("+", 1)[0] != expected:
            issues.append(f"{package}: expected {expected}, installed {actual or 'MISSING'}")
    for package in ("aiortc", "av", "omegaconf", "imageio", "ffmpeg-python"):
        try:
            actual = installed(package)
        except metadata.PackageNotFoundError:
            actual = None
        present[package] = actual
        if actual is None:
            issues.append(f"{package}: not installed")
    # aiortc transport protocol compatibility is bounded in the RT1 CI.
    if present["aiortc"] is not None:
        try:
            major, minor = [int(n) for n in present["aiortc"].split("+")[0].split(".")[:2]]
            if not (major == 1 and minor >= 10):
                issues.append("aiortc: expected >=1.10,<2")
        except (TypeError, ValueError):
            issues.append("aiortc: unparseable version")
    return {
        "schema": "vpr.rt1.musetalk-environment-preflight.v1",
        "python_version": ".".join(map(str, python_version)),
        "issues": issues,
        "dependencies_match_upstream": not issues,
        "cuda_executed": False,
        "production_release_approved": False,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args(argv)
    report = audit()
    print(json.dumps(report, indent=2, sort_keys=True))
    return 2 if report["issues"] else 0


if __name__ == "__main__":
    sys.exit(main())
