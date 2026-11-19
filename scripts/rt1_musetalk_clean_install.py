"""CPU-only, opt-in clean Python 3.10 installation from an APPROVED RT1 wheelhouse.

No network resolver, no GPU import, no release promotion and no implicit write.
Only --execute creates a fresh, private venv. The installed environment is
reported for SEPARATE review: this script must never self-approve its snapshot.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from rt1_musetalk_wheelhouse import verify_wheelhouse

ROOT = Path(__file__).resolve().parent
SCHEMA = "vpr.rt1.offline-clean-install.v1"


def locked_requirements(wheelhouse: Path, manifest: Path, approved_sha256: str):
    """Verify bytes first; then bind each direct file URL to its exact hash."""
    evidence = verify_wheelhouse(wheelhouse, manifest, approved_sha256)
    raw = manifest.read_bytes()
    if hashlib.sha256(raw).hexdigest() != evidence["approved_manifest_sha256"]:
        raise ValueError("approved wheelhouse manifest changed after verification")
    lock = json.loads(raw)
    # Path.as_uri() percent-escapes spaces and shell characters; subprocess
    # receives argv directly and never interprets wheel names as shell code.
    lines = []
    for entry in sorted(lock["wheels"], key=lambda row: row["filename"]):
        wheel = (wheelhouse / entry["filename"]).resolve(strict=True)
        lines.append(f"{wheel.as_uri()} --hash=sha256:{entry['sha256']}")
    return evidence, "\n".join(lines) + "\n"


def check_python310(executable: Path) -> None:
    """Use the exact interpreter chosen by the operator, not PATH python."""
    if not executable.is_absolute() or not executable.is_file():
        raise ValueError("absolute existing Python 3.10 interpreter required")
    result = subprocess.run(
        [str(executable), "-I", "-c",
         "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')"],
        capture_output=True, text=True, check=True, timeout=20,
    )
    if result.stdout.strip() != "3.10":
        raise ValueError("RT1 MuseTalk offline install requires Python 3.10")


def install_approved(
    wheelhouse: Path, manifest: Path, approved_sha256: str,
    python_bin: Path, venv: Path,
) -> dict:
    """Create-only clean venv; install *only* approved hashed wheel URLs.

    Unsuccessful installation leaves the private destination for diagnosis.
    Never recycle it, silently change a lock, or attest production readiness.
    """
    evidence, requirements = locked_requirements(
        wheelhouse, manifest, approved_sha256
    )
    check_python310(python_bin)
    if not venv.is_absolute() or venv.exists() or venv.is_symlink():
        raise ValueError("clean venv destination must be absolute and nonexistent")
    if (not venv.parent.is_dir() or venv.parent.is_symlink()):
        raise ValueError("clean venv parent must be a real existing directory")
    # Exclusive reservation makes a concurrent second installer fail before
    # invoking venv/pip; no destructive --clear or --upgrade flags.
    venv.mkdir(mode=0o700)
    venv_python = venv / "bin" / "python"
    with tempfile.TemporaryDirectory(prefix="vpr-rt1-wheel-lock-") as tmp:
        lock_file = Path(tmp) / "requirements.txt"
        lock_file.write_text(requirements, encoding="utf-8")
        subprocess.run(
            [str(python_bin), "-m", "venv", str(venv)],
            check=True, timeout=120,
        )
        subprocess.run(
            [str(venv_python), "-m", "pip", "install", "--no-index",
             "--no-deps", "--no-cache-dir", "--require-hashes",
             "-r", str(lock_file)],
            check=True, timeout=1800,
        )
    subprocess.run(
        [str(venv_python), "-m", "pip", "check"],
        check=True, timeout=90,
    )
    snapshot = subprocess.run(
        [str(venv_python), str(ROOT / "rt1_musetalk_environment.py"),
         "--emit-snapshot"],
        capture_output=True, text=True, check=True, timeout=90,
    )
    report = json.loads(snapshot.stdout)
    if report.get("issues") or not report.get("environment_snapshot"):
        raise RuntimeError("installed RT1 Python environment was not verified")
    return {
        "schema": SCHEMA,
        "wheel_count": evidence["wheel_count"],
        "wheelhouse_manifest_sha256": evidence["approved_manifest_sha256"],
        "installed_environment_sha256": report["environment_snapshot"]["sha256"],
        "installed_environment_approved": False,
        "gpu_executed": False,
        "production_release_approved": False,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wheelhouse", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--approved-manifest-sha256", required=True)
    parser.add_argument("--python-bin", type=Path)
    parser.add_argument("--venv", type=Path)
    parser.add_argument("--execute", action="store_true",
                        help="explicitly permit creating a fresh venv and installing approved wheels")
    args = parser.parse_args(argv)
    try:
        evidence, requirements = locked_requirements(
            args.wheelhouse, args.manifest, args.approved_manifest_sha256
        )
        if args.execute:
            if args.python_bin is None or args.venv is None:
                parser.error("--execute requires --python-bin and --venv")
            result = install_approved(
                args.wheelhouse, args.manifest,
                args.approved_manifest_sha256, args.python_bin, args.venv
            )
        else:
            result = {
                "schema": SCHEMA,
                "approved_wheel_count": evidence["wheel_count"],
                "install_commands_executed": False,
                "offline_hashed_requirements_sha256": hashlib.sha256(
                    requirements.encode("utf-8")).hexdigest(),
                "gpu_executed": False,
                "production_release_approved": False,
            }
    except (ValueError, OSError, RuntimeError, subprocess.SubprocessError,
            json.JSONDecodeError) as error:
        print(f"RT1 offline clean install BLOCKED: {error}", file=sys.stderr)
        return 2
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
