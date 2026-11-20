"""RT1 public edge is visitor-only, while the canonical loopback lab stays private.

Structural defense in depth; nginx -t and live TLS/browser tests are still
required at deployment. Never treat this as a GPU or public release certificate.
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]
EDGE = ROOT / "deploy/rt1-public-visitor.nginx.conf.template"
MAIN = ROOT / "crates/vpr-owner-lab/src/main.rs"
OWNER_API = ROOT / "crates/vpr-owner-lab/src/http_rt1.rs"
BROWSER = ROOT / "crates/vpr-owner-lab/ui/rt1.js"

STATIC_ROUTES = {
    "/rt1/visit",
    "/rt1.js",
    "/rt1.css",
    "/mic-worklet.js",
    "/vendor/livekit-client.umd.js",
    "/api/rt1/visitor/bootstrap",
}
VISITOR_POST = "(start|answer|ice|text|voice|interrupt|status|close)"
WORKER_ROUTE = f"^/api/rt1/visitor/{VISITOR_POST}$"


class Rt1VisitorPublicEdgeTests(unittest.TestCase):
    def test_only_explicit_visitor_routes_reach_loopback(self):
        config = EDGE.read_text(encoding="utf-8")
        exact = set(re.findall(r"location\s+=\s+([^\s{]+)\s*\{", config))
        regex = re.findall(r"location\s+~\s+([^\s{]+)\s*\{", config)
        self.assertEqual(exact, STATIC_ROUTES)
        self.assertEqual(regex, [WORKER_ROUTE])
        self.assertIn("location / { return 404; }", config)
        self.assertEqual(config.count("proxy_pass http://127.0.0.1:8787;"), 7)
        self.assertEqual(config.count("limit_except GET { deny all; }"), 6)
        self.assertEqual(config.count("limit_except POST { deny all; }"), 1)
        self.assertNotIn("location /api/", config)
        self.assertNotIn("location /rt1/", config)

    def test_https_origin_and_private_host_cannot_be_skipped(self):
        config = EDGE.read_text(encoding="utf-8")
        self.assertIn("listen 443 ssl;", config)
        self.assertIn("ssl_certificate __RT1_TLS_FULLCHAIN_PATH__;", config)
        self.assertIn("ssl_certificate_key __RT1_TLS_PRIVATE_KEY_PATH__;", config)
        self.assertIn('if ($host != "__RT1_VISITOR_FQDN__") { return 444; }', config)
        self.assertIn('if ($http_origin != "https://__RT1_VISITOR_FQDN__") { return 403; }', config)
        self.assertIn('proxy_set_header Host "127.0.0.1:8787";', config)
        self.assertIn('proxy_set_header Origin "http://127.0.0.1:8787";', config)
        self.assertIn('proxy_set_header X-Forwarded-Host "";', config)
        self.assertIn('proxy_set_header Forwarded "";', config)
        self.assertIn("proxy_send_timeout 95s;", config)
        self.assertIn("proxy_read_timeout 95s;", config)

    def test_public_bootstrap_is_not_owner_lab_bootstrap(self):
        main = MAIN.read_text(encoding="utf-8")
        browser = BROWSER.read_text(encoding="utf-8")
        self.assertIn('(&Method::Get, "/api/rt1/visitor/bootstrap") if http_rt1::enabled()', main)
        start = main.index('(&Method::Get, "/api/rt1/visitor/bootstrap")')
        end = main.index("\n        }", start)
        minimal = main[start:end]
        self.assertIn('"csrf_token": state.csrf_token', minimal)
        self.assertNotIn("bootstrap_response(state)", minimal)
        self.assertNotIn("voice_profile", minimal)
        self.assertIn(
            'fetch(visitorOnly ? "/api/rt1/visitor/bootstrap" : "/api/bootstrap"',
            browser,
        )
        self.assertIn('history.replaceState(null, "", window.location.pathname + window.location.search)', browser)
        self.assertIn('const configured = ownerSnapshot?.public_visitor_origin;', browser)
        owner = OWNER_API.read_text(encoding="utf-8")
        self.assertIn('public_origin::configured()', owner)
        validator = (ROOT / "crates/vpr-owner-lab/src/http_rt1_public_origin.rs").read_text(encoding="utf-8")
        self.assertIn('VPR_RT1_PUBLIC_VISITOR_ORIGIN', validator)
        self.assertIn('pub(super) fn configured()', validator)
        self.assertIn('pub(super) fn verified(', validator)
        self.assertIn('RT1_PUBLIC_VISITOR_ORIGIN_INVALID', owner)


if __name__ == "__main__":
    unittest.main()
