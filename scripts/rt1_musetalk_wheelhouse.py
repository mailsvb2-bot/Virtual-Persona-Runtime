"""Offline SHA-256 wheelhouse admission for the RT1 GPU installation candidate.

No installer, network access, package import, CUDA or release promotion.
An independently approved manifest digest is required; never self-approve a
manifest from files in the same wheelhouse during production startup.
"""
from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import stat
import sys
import zipfile

SCHEMA = "vpr.rt1.gpu-wheelhouse-lock.v1"
SHA256 = re.compile(r"[0-9a-f]{64}\Z")
WHEEL_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.+%-]*\.whl\Z")


def verify_wheelhouse(wheelhouse: Path, manifest_path: Path, approved_sha256: str) -> dict:
    """Verify every wheel byte and require an externally approved manifest."""
    if not isinstance(approved_sha256, str) or SHA256.fullmatch(approved_sha256) is None:
        raise ValueError("independent wheelhouse manifest SHA-256 approval required")
    if manifest_path.is_symlink() or not manifest_path.is_file():
        raise ValueError("wheelhouse manifest must be a regular non-symlink file")
    if wheelhouse.is_symlink() or not wheelhouse.is_dir():
        raise ValueError("wheelhouse must be a real directory")
    raw = manifest_path.read_bytes()
    if not raw or len(raw) > 1_000_000:
        raise ValueError("wheelhouse manifest size invalid")
    digest = hashlib.sha256(raw).hexdigest()
    if not hmac.compare_digest(digest, approved_sha256):
        raise ValueError("wheelhouse manifest differs from independent approval")
    try:
        manifest = json.loads(raw)
    except (ValueError, UnicodeError) as error:
        raise ValueError("invalid wheelhouse manifest JSON") from error
    if (not isinstance(manifest, dict) or set(manifest) != {"schema", "python", "wheels"}
            or manifest["schema"] != SCHEMA or manifest["python"] != "3.10"
            or not isinstance(manifest["wheels"], list)
            or not 1 <= len(manifest["wheels"]) <= 1024):
        raise ValueError("unsupported wheelhouse lock manifest")

    expected = {}
    for entry in manifest["wheels"]:
        if not isinstance(entry, dict) or set(entry) != {"filename", "size", "sha256"}:
            raise ValueError("invalid wheel lock entry")
        name, size, checksum = entry["filename"], entry["size"], entry["sha256"]
        if (not isinstance(name, str) or WHEEL_NAME.fullmatch(name) is None
                or type(size) is not int or size <= 0
                or not isinstance(checksum, str) or SHA256.fullmatch(checksum) is None
                or name in expected):
            raise ValueError("ambiguous or malformed wheel lock entry")
        expected[name] = (size, checksum)
    actual = {path.name for path in wheelhouse.iterdir()}
    if actual != set(expected):
        raise ValueError("wheelhouse contains unexpected or missing files")

    for name, (expected_size, expected_digest) in expected.items():
        path = wheelhouse / name
        if path.is_symlink():
            raise ValueError("wheelhouse symlink denied")
        flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
        try:
            fd = os.open(path, flags)
        except OSError as error:
            raise ValueError("wheelhouse file cannot be opened safely") from error
        with os.fdopen(fd, "rb") as handle:
            info = os.fstat(handle.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_size != expected_size:
                raise ValueError("wheelhouse wheel size or file kind mismatch")
            check = hashlib.sha256()
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                check.update(chunk)
            if not hmac.compare_digest(check.hexdigest(), expected_digest):
                raise ValueError("wheelhouse wheel hash mismatch")
            # Check the *same opened file descriptor*, not a path that could
            # be swapped after hashing and before ZIP inspection.
            handle.seek(0)
            if not zipfile.is_zipfile(handle):
                raise ValueError("wheelhouse file is not a ZIP wheel")
    return {
        "schema": "vpr.rt1.gpu-wheelhouse-preflight.v1",
        "python_target": "3.10",
        "wheel_count": len(expected),
        "approved_manifest_sha256": digest,
        "cuda_executed": False,
        "python_packages_installed": False,
        "production_release_approved": False,
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wheelhouse", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--approved-manifest-sha256", required=True)
    args = parser.parse_args(argv)
    try:
        report = verify_wheelhouse(args.wheelhouse, args.manifest,
                                   args.approved_manifest_sha256)
    except (OSError, ValueError) as error:
        print(f"RT1 wheelhouse preflight BLOCKED: {error}", file=sys.stderr)
        return 2
    print(json.dumps(report, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
