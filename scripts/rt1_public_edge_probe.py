#!/usr/bin/env python3
"""Non-mutating HTTPS smoke probe for the existing RT1 visitor-only edge.

No invitation secrets, owner credentials, provider calls or browser sessions.
A network PASS validates the public HTTP boundary ONLY, not GPU, rights,
billing, STOP or production release qualification.
"""
from __future__ import annotations

import argparse
import json
import ssl
import sys
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, newurl):
        return None


@dataclass(frozen=True)
class Probe:
    name: str
    method: str
    path: str
    origin: str | None
    allowed: bool


PROBES = (
    Probe("visitor_page", "GET", "/rt1/visit", None, True),
    Probe("visitor_bootstrap", "GET", "/api/rt1/visitor/bootstrap", None, True),
    Probe("owner_root_denied", "GET", "/", None, False),
    Probe("owner_rt1_denied", "GET", "/rt1", None, False),
    Probe("owner_bootstrap_denied", "GET", "/api/bootstrap", None, False),
    Probe("owner_publish_denied", "POST", "/api/rt1/owner/publish", "same", False),
    Probe("visitor_post_without_origin_denied", "POST", "/api/rt1/visitor/status", None, False),
    Probe("visitor_post_cross_origin_denied", "POST", "/api/rt1/visitor/status", "https://attacker.example.org", False),
    Probe("visitor_post_wrong_method_denied", "GET", "/api/rt1/visitor/start", None, False),
    Probe("worklet", "GET", "/mic-worklet.js", None, True),
)


def exact_https_origin(url: str) -> bool:
    parsed = urllib.parse.urlsplit(url)
    # Reject URL tricks, credentials and explicit ports; require a hostname
    # with nonempty DNS labels and a conventional alphabetic TLD.
    host = parsed.hostname or ""
    return (
        parsed.scheme == "https"
        and parsed.netloc == host
        and "." in host
        and not host.endswith(".")
        and parsed.path == ""
        and not parsed.query
        and not parsed.fragment
        and len(host) <= 253
        and all(
            0 < len(label) <= 63
            and label[0] != "-"
            and label[-1] != "-"
            and all(c.isascii() and (c.isalnum() or c == "-") for c in label)
            for label in host.split(".")
        )
        and len(host.rsplit(".", 1)[-1]) >= 2
        and host.rsplit(".", 1)[-1].isalpha()
        and host.rsplit(".", 1)[-1] not in {"local", "localhost", "invalid", "test", "example"}
    )


def run(origin: str, timeout: float = 7.0) -> dict:
    if not exact_https_origin(origin):
        raise ValueError("expected one exact public https:// DNS origin (no path, port or credentials)")
    opener = urllib.request.build_opener(
        urllib.request.HTTPSHandler(context=ssl.create_default_context()),
        NoRedirect(),
    )
    results = []
    for probe in PROBES:
        headers = {"Accept": "application/json, text/html, application/javascript"}
        if probe.method == "POST":
            headers["Content-Type"] = "application/json"
        if probe.origin == "same":
            headers["Origin"] = origin
        elif probe.origin:
            headers["Origin"] = probe.origin
        request = urllib.request.Request(
            origin + probe.path,
            data=b"{}" if probe.method == "POST" else None,
            headers=headers,
            method=probe.method,
        )
        try:
            with opener.open(request, timeout=timeout) as response:
                status = response.status
                body = response.read(512 * 1024 + 1)
        except urllib.error.HTTPError as error:
            status = error.code
            body = error.read(512 * 1024 + 1)
        except Exception as error:
            results.append({"probe": probe.name, "pass": False, "error_class": type(error).__name__})
            continue
        valid = 200 <= status < 300 if probe.allowed else 400 <= status < 500
        if probe.name == "visitor_page":
            valid = valid and b'id="visitor-panel"' in body and len(body) <= 512 * 1024
        if probe.name == "visitor_bootstrap":
            try:
                payload = json.loads(body)
                valid = valid and set(payload) == {"csrf_token"} and isinstance(payload["csrf_token"], str)
            except (ValueError, TypeError, KeyError):
                valid = False
        if probe.name == "worklet":
            valid = valid and len(body) <= 512 * 1024 and b"registerProcessor" in body
        results.append({"probe": probe.name, "http_status": status, "pass": valid})
    return {
        "schema": "rt1-public-edge-readonly-probe-v1",
        "origin": origin,
        "public_http_boundary_pass": all(item["pass"] for item in results),
        "gpu_accepted": False,
        "rights_budget_accepted": False,
        "production_release_accepted": False,
        "checks": results,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--origin", required=True, help="Approved HTTPS visitor DNS origin")
    parser.add_argument("--timeout", type=float, default=7.0)
    args = parser.parse_args()
    try:
        report = run(args.origin, args.timeout)
    except ValueError as error:
        parser.error(str(error))
    print(json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2))
    return 0 if report["public_http_boundary_pass"] else 2


if __name__ == "__main__":
    sys.exit(main())
