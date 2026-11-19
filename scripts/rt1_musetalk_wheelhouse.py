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


def _validate_wheel_archive(handle) -> None:
    """Reject unsafe wheel ZIP members without extracting or running code.

    A digest pins bytes, not archive semantics. The independent wheel review
    must never approve traversal, duplicate ZIP names, symlinks or a ZIP with
    no Python wheel metadata. Full package installation remains a separate gate.
    """
    try:
        with zipfile.ZipFile(handle) as archive:
            members = archive.infolist()
            if not members or len(members) > 50000:
                raise ValueError("wheelhouse ZIP member count invalid")
            seen = set()
            names = set()
            total_size = 0
            for member in members:
                name = member.filename
                if (not name or "\\" in name or name.startswith("/")
                        or any(part in ("", ".", "..") for part in
                               name.rstrip("/").split("/"))
                        or len(name) > 4096):
                    raise ValueError("unsafe wheelhouse ZIP member path")
                if name in seen:
                    raise ValueError("duplicate wheelhouse ZIP member")
                seen.add(name)
                if member.flag_bits & 1:
                    raise ValueError("encrypted wheelhouse ZIP member")
                unix_mode = (member.external_attr >> 16) & 0o170000
                if unix_mode not in (0, 0o100000, 0o040000):
                    raise ValueError("non-regular wheelhouse ZIP member")
                total_size += member.file_size
                if total_size > 32 * 1024**3:
                    raise ValueError("wheelhouse ZIP expanded size exceeds limit")
                if not member.is_dir():
                    names.add(name)
            wheels = [name for name in names if name.endswith(".dist-info/WHEEL")]
            if len(wheels) != 1:
                raise ValueError("wheelhouse Python WHEEL metadata missing or ambiguous")
            prefix = wheels[0][:-len("WHEEL")]
            if prefix + "METADATA" not in names or prefix + "RECORD" not in names:
                raise ValueError("wheelhouse package metadata or RECORD missing")
            # Metadata is small in a valid wheel. Read it to verify ZIP CRC and
            # avoid accepting a ZIP whose central directory merely names WHEEL.
            item = archive.getinfo(wheels[0])
            if item.file_size > 65536:
                raise ValueError("wheelhouse WHEEL metadata too large")
            with archive.open(item) as content:
                wheel_metadata = content.read(65537)
            if not any(line.startswith(b"Wheel-Version: ") for line in wheel_metadata.splitlines()):
                raise ValueError("wheelhouse WHEEL metadata invalid")
    except (zipfile.BadZipFile, RuntimeError, EOFError) as error:
        raise ValueError("wheelhouse ZIP archive invalid") from error


def _wheel_digest(path: Path) -> tuple[int, str]:
    """Hash and validate one wheel using a single non-followed descriptor."""
    if path.is_symlink():
        raise ValueError("wheelhouse symlink denied")
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    try:
        fd = os.open(path, flags)
    except OSError as error:
        raise ValueError("wheelhouse file cannot be opened safely") from error
    with os.fdopen(fd, "rb") as handle:
        info = os.fstat(handle.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_size <= 0:
            raise ValueError("wheelhouse wheel size or file kind mismatch")
        check = hashlib.sha256()
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            check.update(chunk)
        handle.seek(0)
        if not zipfile.is_zipfile(handle):
            raise ValueError("wheelhouse file is not a ZIP wheel")
        handle.seek(0)
        _validate_wheel_archive(handle)
    return info.st_size, check.hexdigest()


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
        size, actual_digest = _wheel_digest(wheelhouse / name)
        if size != expected_size:
            raise ValueError("wheelhouse wheel size or file kind mismatch")
        if not hmac.compare_digest(actual_digest, expected_digest):
            raise ValueError("wheelhouse wheel hash mismatch")
    return {
        "schema": "vpr.rt1.gpu-wheelhouse-preflight.v1",
        "python_target": "3.10",
        "wheel_count": len(expected),
        "approved_manifest_sha256": digest,
        "cuda_executed": False,
        "python_packages_installed": False,
        "production_release_approved": False,
    }


def draft_manifest(wheelhouse: Path, output: Path) -> dict:
    """Create an unapproved lock candidate; caller must approve independently."""
    if wheelhouse.is_symlink() or not wheelhouse.is_dir():
        raise ValueError("wheelhouse must be a real directory")
    if output.exists() or output.is_symlink():
        raise ValueError("refusing to overwrite wheelhouse lock candidate")
    files = sorted(wheelhouse.iterdir(), key=lambda p: p.name)
    if not 1 <= len(files) <= 1024:
        raise ValueError("wheelhouse count invalid")
    wheels = []
    for path in files:
        if WHEEL_NAME.fullmatch(path.name) is None:
            raise ValueError("wheelhouse contains non-wheel or invalid filename")
        size, digest = _wheel_digest(path)
        wheels.append({"filename": path.name, "size": size, "sha256": digest})
    payload = {"schema": SCHEMA, "python": "3.10", "wheels": wheels}
    raw = (json.dumps(payload, sort_keys=True, indent=2) + "\n").encode("utf-8")
    # Exclusive creation preserves earlier reviewed evidence.
    with output.open("xb") as handle:
        handle.write(raw)
        handle.flush()
        os.fsync(handle.fileno())
    return {
        "draft_sha256_for_independent_review": hashlib.sha256(raw).hexdigest(),
        "wheel_count": len(wheels),
        "approved": False,
        "cuda_executed": False,
        "production_release_approved": False,
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wheelhouse", required=True, type=Path)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--approved-manifest-sha256")
    parser.add_argument("--draft-manifest-output", type=Path)
    args = parser.parse_args(argv)
    if args.draft_manifest_output:
        if args.manifest or args.approved_manifest_sha256:
            parser.error("draft and approved verification are mutually exclusive")
    elif args.manifest is None or args.approved_manifest_sha256 is None:
        parser.error("--manifest and --approved-manifest-sha256 are required")
    try:
        if args.draft_manifest_output:
            report = draft_manifest(args.wheelhouse, args.draft_manifest_output)
        else:
            report = verify_wheelhouse(args.wheelhouse, args.manifest,
                                       args.approved_manifest_sha256)
    except (OSError, ValueError) as error:
        print(f"RT1 wheelhouse preflight BLOCKED: {error}", file=sys.stderr)
        return 2
    print(json.dumps(report, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
