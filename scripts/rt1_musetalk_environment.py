"""RT1 GPU runtime dependency audit without importing GPU libraries.

The tested upstream MuseTalk revision prescribes Python 3.10 + torch 2.0.1
(CUDA 11.8 PyTorch wheel index) and the versions from requirements.txt.
This audit does not grant GPU or production readiness.
"""
from __future__ import annotations

import argparse
import hashlib
import hmac
from importlib import metadata
import json
import re
import subprocess
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


def environment_snapshot(*, python_version=None, packages=None):
    """Exact normalized installed-distribution inventory (no imports/CUDA).

    An approved fingerprint must be held outside this environment. Versions
    alone are not proof of wheel-file contents, package licensing or CUDA ABI.
    """
    version = tuple(python_version or sys.version_info[:2])
    if len(version) != 2 or any(type(v) is not int or v < 0 for v in version):
        raise ValueError("invalid Python version for environment snapshot")
    if packages is None:
        packages = [
            (dist.metadata.get("Name"), dist.version)
            for dist in metadata.distributions()
        ]
    registry = {}
    for raw_name, raw_version in packages:
        if (not isinstance(raw_name, str) or not isinstance(raw_version, str)
                or not raw_name.strip() or not raw_version.strip()
                or len(raw_name) > 256 or len(raw_version) > 256):
            raise ValueError("invalid installed distribution metadata")
        name = re.sub(r"[-_.]+", "-", raw_name).lower()
        if re.fullmatch(r"[a-z0-9][a-z0-9-]*", name) is None:
            raise ValueError("invalid installed distribution name")
        if name in registry:
            raise ValueError(f"ambiguous duplicate distribution: {name}")
        registry[name] = raw_version
    if not registry:
        raise ValueError("empty installed distribution inventory")
    payload = {
        "schema": "vpr.rt1.python-distribution-snapshot.v1",
        "python": f"{version[0]}.{version[1]}",
        "packages": [{"name": name, "version": registry[name]} for name in sorted(registry)],
    }
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    payload["sha256"] = hashlib.sha256(encoded).hexdigest()
    return payload


def verify_approved_environment(expected_digest):
    """Deny unapproved dependency drift before importing any CUDA libraries."""
    if (not isinstance(expected_digest, str)
            or re.fullmatch(r"[0-9a-f]{64}", expected_digest) is None):
        raise RuntimeError("RT1_MUSETALK_ENVIRONMENT_SHA256 approval required")
    report = audit()
    if report["issues"]:
        raise RuntimeError("MuseTalk installed Python dependencies do not match approved recipe")
    try:
        actual = environment_snapshot()["sha256"]
    except ValueError as error:
        raise RuntimeError("MuseTalk installed distribution inventory invalid") from error
    if not hmac.compare_digest(expected_digest, actual):
        raise RuntimeError("MuseTalk installed distribution snapshot differs from approved pin")
    # A pinned inventory is not proof that the installed dependency metadata is
    # internally compatible. Check the resolver's own dependency requirements
    # before any import of CUDA/model code, including direct worker startup.
    try:
        subprocess.run(
            [sys.executable, "-m", "pip", "check"], check=True,
            capture_output=True, timeout=30,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise RuntimeError("MuseTalk installed dependency graph failed pip check") from error
    return actual


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--emit-snapshot", action="store_true",
                        help="print exact installed-distribution fingerprint for independent approval")
    parser.add_argument("--approved-snapshot-sha256",
                        help="fail unless the entire installed distribution set matches this pin")
    args = parser.parse_args(argv)
    report = audit()
    if args.emit_snapshot or args.approved_snapshot_sha256:
        try:
            snapshot = environment_snapshot()
            report["environment_snapshot"] = snapshot
            if args.approved_snapshot_sha256:
                verify_approved_environment(args.approved_snapshot_sha256)
                report["environment_snapshot_approved"] = True
            else:
                report["environment_snapshot_approved"] = False
        except (ValueError, RuntimeError) as error:
            report["issues"].append(str(error))
    print(json.dumps(report, indent=2, sort_keys=True))
    return 2 if report["issues"] else 0


if __name__ == "__main__":
    sys.exit(main())
