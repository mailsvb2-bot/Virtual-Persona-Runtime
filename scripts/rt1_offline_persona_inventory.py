#!/usr/bin/env python3
"""Offline, metadata-only RT0 reviewed Persona migration inventory.

NEVER validates Persona claims or authorizes RT1 import. Prints no raw material.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

SCHEMAS = {"vpr-reviewed-owner-persona-1", "vpr-reviewed-owner-persona-2"}


def inspect(path: Path) -> dict:
    result = {
        "schema": "rt1-offline-persona-inventory-0.1",
        "source_schema": "UNKNOWN",
        "classification": "REJECT",
        "automatic_migration_allowed": False,
        "owner_review_required": True,
        "contains_live_session_authority": False,
        "reason": "INVALID_OR_UNSUPPORTED_STORE",
    }
    try:
        if not path.is_file() or path.stat().st_size > 16 * 1024 * 1024:
            return result
        raw = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(raw, dict) or set(raw) != {"schema_version", "snapshot"}:
            return result
        schema = raw.get("schema_version")
        snapshot = raw.get("snapshot")
        if schema not in SCHEMAS or not isinstance(snapshot, dict):
            return result
        if not isinstance(snapshot.get("persona_id"), str) or not snapshot["persona_id"].strip():
            return result
        if type(snapshot.get("persona_version")) is not int or snapshot["persona_version"] < 2:
            return result
        claims = snapshot.get("claims")
        if not isinstance(claims, list) or not claims:
            return result
        # Shape checks only. Canonical Rust owner-context validation remains authoritative.
        if schema == "vpr-reviewed-owner-persona-2":
            if not all(
                isinstance(c, dict)
                and isinstance(c.get("claim_id"), str)
                and type(c.get("history_complete")) is bool
                and isinstance(c.get("revisions"), list)
                and bool(c["revisions"])
                for c in claims
            ):
                return result
        else:
            if not all(isinstance(c, dict) and isinstance(c.get("claim_id"), str)
                       and type(c.get("revision")) is int for c in claims):
                return result
        result.update(
            source_schema=schema,
            classification="OWNER_REVIEW_ONLY",
            reason=("V1_HISTORY_PARTIAL_REQUIRES_OWNER_REVIEW" if schema.endswith("-1")
                    else "V2_EXACT_HISTORY_REQUIRES_OWNER_REVIEW"),
        )
        return result
    except (OSError, UnicodeError, ValueError, TypeError):
        return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("store_file", type=Path, help="explicit local reviewed-Persona JSON; not a credential-store export")
    args = parser.parse_args()
    print(json.dumps(inspect(args.store_file), indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
