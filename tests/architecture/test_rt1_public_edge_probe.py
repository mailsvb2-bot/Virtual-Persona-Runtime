"""No-network tests for the RT1 external-HTTPS visitor edge probe."""
import io
import json
import sys
from pathlib import Path
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[2] / "scripts"
sys.path.insert(0, str(SCRIPTS))
import rt1_public_edge_probe as probe


class PublicEdgeProbeTests(unittest.TestCase):
    def test_origin_rejects_private_unsafe_or_path_alias(self):
        self.assertTrue(probe.exact_https_origin("https://persona.example.org"))
        for url in (
            "http://persona.example.org",
            "https://localhost",
            "https://127.0.0.1",
            "https://persona.example.org:443",
            "https://me@persona.example.org",
            "https://persona.example.org/rt1/visit",
            "https://persona.example.org/?x=1",
            "https://persona.example.org#token",
            "https://persona..org",
            "https://-persona.example.org",
            "https://persona.example.org.evil.org/path",
        ):
            with self.subTest(url=url):
                self.assertFalse(probe.exact_https_origin(url))

    def test_backend_csrf_denial_cannot_fake_nginx_edge_protection(self):
        class Response(io.BytesIO):
            def __init__(self, status, body):
                super().__init__(body)
                self.status = status

        class FakeOpener:
            leaked_owner = False
            leaked_origin = False
            csrf_forwarded = False

            def open(self, request, timeout):
                path = request.full_url.split("persona.example.org", 1)[1]
                csrf = dict(request.header_items()).get("X-vpr-csrf")
                if request.get_method() == "POST":
                    self.csrf_forwarded = self.csrf_forwarded or bool(csrf)
                if path == "/api/rt1/visitor/bootstrap":
                    return Response(200, json.dumps({"csrf_token": "s" * 64}).encode())
                if path == "/rt1/visit":
                    return Response(200, b'<section id="visitor-panel">visitor</section>')
                if path == "/mic-worklet.js":
                    return Response(200, b'registerProcessor("vpr", fn)')
                if path == "/api/rt1/owner/publish":
                    if self.leaked_owner:
                        return Response(403, b'{"code":"CSRF_DENIED"}')
                    return Response(404, b"<html>Not Found</html>")
                if path == "/api/rt1/visitor/status":
                    header = dict(request.header_items())
                    if header.get("Origin") == "https://persona.example.org":
                        # The canonical backend denies this nonexistent
                        # session, proving the visitor POST was forwarded.
                        return Response(409, b'{"ok":false,"code":"RT1_VISITOR_REVOKED"}')
                    if self.leaked_origin:
                        return Response(403, b'{"code":"CSRF_DENIED"}')
                    return Response(403, b"<html>Forbidden</html>")
                if path == "/api/rt1/visitor/start":
                    return Response(403, b"<html>Forbidden</html>")
                return Response(404, b"<html>Not Found</html>")

        fake = FakeOpener()
        with patch.object(probe.urllib.request, "build_opener", return_value=fake):
            good = probe.run("https://persona.example.org")
            self.assertTrue(good["public_http_boundary_pass"])
            self.assertTrue(fake.csrf_forwarded)
            fake.leaked_owner = True
            bad_owner = probe.run("https://persona.example.org")
            self.assertFalse(bad_owner["public_http_boundary_pass"])
            self.assertFalse(next(p for p in bad_owner["checks"] if p["probe"] == "owner_publish_denied")["pass"])
            fake.leaked_owner = False
            fake.leaked_origin = True
            bad_origin = probe.run("https://persona.example.org")
            self.assertFalse(bad_origin["public_http_boundary_pass"])
            self.assertFalse(next(p for p in bad_origin["checks"] if p["probe"] == "visitor_post_cross_origin_denied")["pass"])

    def test_only_required_visitor_endpoints_may_be_successful(self):
        allowed = {p.path for p in probe.PROBES if p.allowed}
        self.assertEqual(allowed, {
            "/rt1/visit", "/api/rt1/visitor/bootstrap", "/mic-worklet.js",
        })
        self.assertTrue(all(not p.path.startswith("/api/rt1/owner/") or not p.allowed
                            for p in probe.PROBES))
        for item in probe.PROBES:
            self.assertTrue(item.path.startswith("/"))
        self.assertIn(
            ("POST", "/api/rt1/visitor/status", "same"),
            {(p.method, p.path, p.origin) for p in probe.PROBES},
        )
        self.assertIn(
            ("POST", "/api/rt1/visitor/status", None),
            {(p.method, p.path, p.origin) for p in probe.PROBES},
        )
        self.assertIn(
            ("POST", "/api/rt1/visitor/status", "https://attacker.example.org"),
            {(p.method, p.path, p.origin) for p in probe.PROBES},
        )


if __name__ == "__main__":
    unittest.main()
