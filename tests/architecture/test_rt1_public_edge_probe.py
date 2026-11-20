"""No-network tests for the RT1 external-HTTPS visitor edge probe."""
import sys
from pathlib import Path
import unittest

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
            ("POST", "/api/rt1/visitor/status", None),
            {(p.method, p.path, p.origin) for p in probe.PROBES},
        )
        self.assertIn(
            ("POST", "/api/rt1/visitor/status", "https://attacker.example.org"),
            {(p.method, p.path, p.origin) for p in probe.PROBES},
        )


if __name__ == "__main__":
    unittest.main()
