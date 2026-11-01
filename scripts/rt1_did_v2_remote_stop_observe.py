#!/usr/bin/env python3
"""Observe a real D-ID V2 session without confusing telemetry with STOP proof.

Run against an operator-created session while a second, independent viewer
records received audio/video after canonical owner revoke. No token or payload
is printed or stored. This never requests a new session or changes provider state.
"""
import argparse
import hashlib
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

API = "https://api.d-id.com/v2/agents/sessions/"
MAX_BYTES = 1_000_000


def read_session(session_id: str, credential: str) -> tuple[int, dict]:
    url = API + urllib.parse.quote(session_id, safe="")
    req = urllib.request.Request(url, headers={
        "Authorization": "Basic " + credential,
        "Accept": "application/json",
    })
    try:
        with urllib.request.urlopen(req, timeout=12) as response:
            raw = response.read(MAX_BYTES + 1)
            status = response.status
    except urllib.error.HTTPError as exc:
        # Never expose provider error bodies: they can contain identifying data.
        return exc.code, {}
    except (urllib.error.URLError, TimeoutError):
        return 0, {}
    if len(raw) > MAX_BYTES:
        return 0, {}
    try:
        obj = json.loads(raw)
    except (ValueError, UnicodeError):
        return status, {}
    return status, obj if isinstance(obj, dict) else {}


def scalar(obj: dict, name: str):
    value = obj.get(name)
    return value if type(value) in (bool, int, float, str) and len(str(value)) <= 80 else None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--session-id", required=True)
    parser.add_argument("--interval", type=int, default=5)
    parser.add_argument("--samples", type=int, default=6)
    parser.add_argument("--output", help="Optional sanitized JSON evidence path")
    args = parser.parse_args()
    credential = os.environ.get("VPR_DID_API_KEY", "")
    if not credential:
        print("BLOCKED: VPR_DID_API_KEY is missing", file=sys.stderr)
        return 2
    if not (1 <= args.interval <= 30 and 2 <= args.samples <= 30):
        parser.error("interval 1..30 seconds; samples 2..30")
    if not args.session_id or "/" in args.session_id or len(args.session_id) > 256:
        parser.error("invalid session ID")
    fingerprint = hashlib.sha256(args.session_id.encode()).hexdigest()[:16]
    print("Observe independent viewer before and after owner revoke; enter no tokens here.", file=sys.stderr)
    results = []
    for i in range(args.samples):
        if i:
            time.sleep(args.interval)
        code, body = read_session(args.session_id, credential)
        # The provider schema may evolve. Never serialize arbitrary response
        # fields, participant details, tokens, transcripts, or media URLs.
        row = {
            "sample": i + 1,
            "utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "http_status": code,
            "provider_status": scalar(body, "status"),
            "ended_at_present": bool(body.get("ended_at")),
            "closed_at_present": bool(body.get("closed_at")),
        }
        results.append(row)
        print(json.dumps(row, ensure_ascii=False))
    document = {
        "schema": "vpr-did-v2-readonly-observation-1",
        "session_sha256_prefix": fingerprint,
        "provider": "D-ID V2",
        "endpoint": "/v2/agents/sessions/{sessionId}",
        "samples": results,
        "remote_video_stop_confirmed": False,
        "release_ready": False,
        "warning": "GET telemetry and viewer disconnect do not prove provider video termination. Independent receiver RTP evidence and authenticated provider confirmation are mandatory.",
    }
    if args.output:
        with open(args.output, "x", encoding="utf-8") as out:
            json.dump(document, out, indent=2, ensure_ascii=False)
            out.write("\n")
    print("RESULT: REMOTE_VIDEO_STOP_UNCONFIRMED (read-only provider observations)", file=sys.stderr)
    return 3


if __name__ == "__main__":
    sys.exit(main())
