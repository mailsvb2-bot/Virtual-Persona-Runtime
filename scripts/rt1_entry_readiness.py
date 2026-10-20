#!/usr/bin/env python3
"""Read-only RT1 entry status. Never promotes release maturity from owner sign-off.

This utility deliberately cannot certify formal RT0 exit: only the canonical
exact-evidence verifier may do that. Its purpose is to make the distinction
between the owner's accepted working journey and release qualification visible.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OWNER_RECORD = ROOT / "docs/release-evidence/rt0/OWNER_FUNCTIONAL_ACCEPTANCE.md"
RT1_SPEC = ROOT / "docs/releases/RT1_RELEASE_SPEC.md"


def status(root: Path = ROOT) -> dict:
    owner_record = root / "docs/release-evidence/rt0/OWNER_FUNCTIONAL_ACCEPTANCE.md"
    rt1_spec = root / "docs/releases/RT1_RELEASE_SPEC.md"
    owner_text = owner_record.read_text(encoding="utf-8") if owner_record.is_file() else ""
    spec_text = rt1_spec.read_text(encoding="utf-8") if rt1_spec.is_file() else ""
    owner_accepted = "**Owner decision: R0 ACCEPTED / CLOSED for the agreed functional demonstration.**" in owner_text
    entry_blocked = (
        "**Status:** `BLOCKED_ON_RT0_EXIT`" in spec_text
        and "**Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`" in spec_text
    )
    # This checker is not an exit verifier, so it MUST NOT infer success from
    # test green badges, acceptance documents, or existence of generated files.
    return {
        "schema": "rt1-entry-readiness-0.1",
        "owner_functional_r0_accepted": owner_accepted,
        "formal_rt0_exit_verified": False,
        "rt1_full_implementation_authorized": False,
        "allowed_now": ["BOUNDED_FEASIBILITY_SPIKE_ONLY"] if entry_blocked else [],
        "rt1_release_spec_consistent": entry_blocked,
        "reason": "FORMAL_RT0_EXIT_NOT_VERIFIED" if entry_blocked else "RT1_SPEC_ENTRY_CONTRACT_UNRECOGNIZED",
        "next_gate": "canonical exact-candidate vpr-rt0-exit-evidence pass and reviewed RT0 evidence bundle",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=ROOT, help="repository root")
    args = parser.parse_args()
    result = status(args.repo.resolve())
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0 if result["rt1_release_spec_consistent"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
