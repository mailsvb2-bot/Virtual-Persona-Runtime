#!/usr/bin/env python3
"""Read-only RT1 candidate evidence inventory tied to exact source digests.

This does not manufacture production evidence. Missing external attestations
always remain UNVERIFIED regardless of whether synthetic CI tests are green.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def git_head() -> str:
    try:
        return subprocess.check_output(
            ["git", "-C", str(ROOT), "rev-parse", "HEAD"],
            text=True, stderr=subprocess.DEVNULL,
        ).strip()
    except (OSError, subprocess.CalledProcessError):
        return "UNKNOWN"


def inventory() -> dict:
    spec = ROOT / "docs/releases/RT1_RELEASE_SPEC.md"
    rights_path = ROOT / "contracts/rt1-consent-rights-0.1.json"
    budget_path = ROOT / "contracts/rt1-cost-budget-0.1.json"
    rights = json.loads(rights_path.read_text(encoding="utf-8"))
    budget = json.loads(budget_path.read_text(encoding="utf-8"))
    edge = ROOT / "deploy/rt1-public-visitor.nginx.conf.template"
    public_probe = ROOT / "scripts/rt1_public_edge_probe.py"
    gpu_runner = ROOT / "scripts/rt1_musetalk_hardware_acceptance.sh"
    # Source code and local fixture tests are NOT evidence of actual Internet
    # deployment, NVIDIA inference, independent tenant isolation or cost data.
    blockers = [
        {
            "id": "P0_MULTI_OWNER_ACCOUNTS",
            "status": "UNVERIFIED",
            "reason": "Canonical durable owner workspace currently supports one reviewed Persona; tenant isolation is not accepted",
            "source": "crates/vpr-owner-lab/src/rt1_workspace_service.rs",
        },
        {
            "id": "P0_PUBLIC_HTTPS_DEPLOYMENT",
            "status": "UNVERIFIED",
            "reason": "Visitor-only HTTPS edge implemented in source, but no exact-host TLS/live deny-path audit supplied",
            "source": str(edge.relative_to(ROOT)),
            "probe_available": public_probe.is_file(),
        },
        {
            "id": "P0_REAL_GPU_MUSETALK",
            "status": "UNVERIFIED",
            "reason": "Hardware acceptance harness exists; no verified approved NVIDIA face/audio run bound to candidate",
            "source": str(gpu_runner.relative_to(ROOT)),
            "hardware_harness_available": gpu_runner.is_file(),
        },
        {
            "id": "P1_RIGHTS_CONSENT",
            "status": "UNVERIFIED",
            "reason": "Durable scoped biometric rights and commercial consent runtime not implemented",
            "source": str(rights_path.relative_to(ROOT)),
            "runtime_implemented": rights["durable_consent_rights_state_implemented"],
        },
        {
            "id": "P1_COST_BUDGET",
            "status": "UNVERIFIED",
            "reason": "Prepublication estimate, concurrency-safe reservation and provider usage budget runtime not implemented",
            "source": str(budget_path.relative_to(ROOT)),
            "prepublication_estimate_implemented": budget["prepublication_estimate_runtime_implemented"],
            "hard_budget_implemented": budget["hard_session_budget_runtime_implemented"],
        },
        {
            "id": "P1_EXACT_CANDIDATE_RELEASE_EVIDENCE",
            "status": "UNVERIFIED",
            "reason": "Full real-device, rights, cost, GPU, A/V sync, STOP and rollback attestations need independent review",
            "source": str(spec.relative_to(ROOT)),
        },
    ]
    return {
        "schema": "rt1-release-candidate-inventory-v1",
        "candidate_git_sha": git_head(),
        "spec_sha256": digest(spec),
        "rights_contract_sha256": digest(rights_path),
        "budget_contract_sha256": digest(budget_path),
        "local_owner_visitor_fixture_tests": "AVAILABLE_NOT_PRODUCTION_EVIDENCE",
        "production_ready": False,
        "stop_condition": "DO_NOT_PUBLICLY_PUBLISH_OR_CHARGE_UNTIL_VERIFIED",
        "requirements": blockers,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--require-ready", action="store_true",
                        help="Fail closed while verified real-world release proof is absent")
    parser.add_argument("--output", type=Path, help="Optional output file; never overwrites")
    args = parser.parse_args()
    report = inventory()
    encoded = json.dumps(report, indent=2, ensure_ascii=False, sort_keys=True) + "\n"
    if args.output:
        with args.output.open("x", encoding="utf-8") as out:
            out.write(encoded)
    else:
        print(encoded, end="")
    return 2 if args.require_ready and not report["production_ready"] else 0


if __name__ == "__main__":
    sys.exit(main())
