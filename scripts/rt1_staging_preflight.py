#!/usr/bin/env python3
"""Connect existing RT1 install, GPU, worker and HTTPS checks without promotion.

Run on the approved staging host with the exact worker Python interpreter.
No model downloads, installation, publication, provider CREATE or release bypass.
The operator starts the existing worker and Owner Lab, never another runtime.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import urllib.error
import urllib.request

from rt1_public_edge_probe import NoRedirect, exact_https_origin, run as probe_edge
from rt1_musetalk_clean_install import locked_requirements

ROOT = Path(__file__).resolve().parents[1]


def required(name: str) -> str:
    value = os.environ.get(name, "")
    if not value or value != value.strip() or any(ord(c) < 32 for c in value):
        raise ValueError("missing or malformed " + name)
    return value


def configuration() -> tuple[str, str, str]:
    if required("RT1_VIDEO_RENDERER") != "musetalk-v15":
        raise ValueError("real MuseTalk renderer required")
    if required("VPR_RT1_LOCAL_EXPERIMENT") != "true":
        raise ValueError("explicit staging experiment required")
    if required("VPR_RT1_CANONICAL_VOICE_WORKER") != "true":
        raise ValueError("canonical voice worker required")
    if os.environ.get("VPR_RT1_DIAGNOSTIC_RTP", "false") != "false":
        raise ValueError("diagnostic RTP forbidden")
    port = os.environ.get("RT1_WORKER_PORT", "8099")
    if not port.isascii() or not port.isdecimal() or not 1 <= int(port) <= 65535:
        raise ValueError("invalid worker port")
    endpoint = required("RT1_WORKER_ENDPOINT")
    if endpoint != f"http://127.0.0.1:{int(port)}":
        raise ValueError("worker and Owner Lab endpoint must match loopback listener")
    origin = required("VPR_RT1_PUBLIC_VISITOR_ORIGIN")
    if not exact_https_origin(origin):
        raise ValueError("approved exact HTTPS visitor origin required")
    token = required("RT1_WORKER_TOKEN")
    if len(token) < 32:
        raise ValueError("staging worker token must have at least 32 characters")
    for name in ("RT1_WORKER_DB", "VPR_OWNER_LAB_PERSONA_STORE_PATH"):
        if not Path(required(name)).is_absolute():
            raise ValueError(name + " must be an absolute persistent path")
    return endpoint, origin, token


def worker_probe(endpoint: str, token: str) -> bool:
    """Use the existing read-only operation endpoint; never allocate media."""
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    url = endpoint + "/v1/avatar/operations"
    try:
        with opener.open(url, timeout=5):
            return False
    except urllib.error.HTTPError as error:
        if error.code != 401:
            return False
    request = urllib.request.Request(url, headers={"Authorization": "Bearer " + token})
    with opener.open(request, timeout=5) as response:
        raw = response.read(65537)
        if response.status != 200 or len(raw) > 65536:
            return False
    payload = json.loads(raw)
    # A staging launch with unresolved operations needs reconciliation first.
    return isinstance(payload, dict) and payload.get("operations") == []


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate-sha", required=True)
    parser.add_argument("--gpu-evidence", type=Path, required=True,
                        help="new absolute destination for real GPU acceptance")
    args = parser.parse_args(argv)
    report = {"schema": "vpr.rt1.staging-preflight.v1",
              "production_ready": False, "browser_av_stop_verified": False,
              "checks": {}}
    try:
        if not re.fullmatch(r"[0-9a-f]{40}", args.candidate_sha):
            raise ValueError("exact candidate SHA required")
        def git(*parts):
            return subprocess.check_output(["git", "-C", str(ROOT), *parts],
                                           text=True, timeout=15).strip()
        if git("rev-parse", "HEAD") != args.candidate_sha or git("status", "--porcelain"):
            raise ValueError("candidate checkout mismatch or dirty checkout")
        endpoint, origin, token = configuration()
        report["candidate_sha"] = args.candidate_sha
        report["checks"]["configuration"] = True
        _, requirements = locked_requirements(
            Path(required("RT1_MUSETALK_WHEELHOUSE_DIR")),
            Path(required("RT1_MUSETALK_WHEELHOUSE_MANIFEST")),
            required("RT1_MUSETALK_WHEELHOUSE_MANIFEST_SHA256"),
        )
        report["checks"]["approved_install_lock_sha256"] = hashlib.sha256(
            requirements.encode("utf-8")).hexdigest()
        if not args.gpu_evidence.is_absolute() or args.gpu_evidence.exists() or args.gpu_evidence.is_symlink():
            raise ValueError("new absolute GPU evidence path required")
        # Reuse the existing complete wheelhouse/environment/source/model/GPU
        # harness, with the same interpreter that will run the canonical worker.
        env = os.environ.copy()
        env["PATH"] = str(Path(sys.executable).parent) + os.pathsep + env.get("PATH", "")
        subprocess.run(["bash", str(ROOT / "scripts/rt1_musetalk_hardware_acceptance.sh"),
                        str(args.gpu_evidence)], env=env, check=True, timeout=1800,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        evidence = json.loads(args.gpu_evidence.read_text(encoding="utf-8"))
        if (evidence.get("schema") != "vpr.rt1.real-gpu-compute-acceptance.v1"
                or evidence.get("real_cuda_execution") is not True
                or evidence.get("meets_compute_deadline") is not True):
            raise ValueError("GPU compute acceptance not established")
        report["checks"]["gpu_compute"] = True
        report["checks"]["gpu_evidence_sha256"] = hashlib.sha256(args.gpu_evidence.read_bytes()).hexdigest()
        report["checks"]["worker_authenticated_empty_ledger"] = worker_probe(endpoint, token)
        report["checks"]["https_boundary"] = probe_edge(origin)["public_http_boundary_pass"]
        passed = all(report["checks"][key] is True for key in (
            "configuration", "gpu_compute", "worker_authenticated_empty_ledger", "https_boundary"))
        report["staging_preflight_pass"] = passed
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        # Never emit provider payloads, credentials or arbitrary exception text.
        report["staging_preflight_pass"] = False
        report["error_class"] = type(error).__name__
    print(json.dumps(report, sort_keys=True))
    return 0 if report["staging_preflight_pass"] else 2


if __name__ == "__main__":
    sys.exit(main())
