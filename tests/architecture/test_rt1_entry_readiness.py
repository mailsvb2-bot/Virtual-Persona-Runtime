"""RT1 pre-entry readiness is explicitly non-promoting (no provider charges)."""
from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[2] / "scripts/rt1_entry_readiness.py"
spec = importlib.util.spec_from_file_location("rt1_entry_readiness", SCRIPT)
module = importlib.util.module_from_spec(spec)
assert spec and spec.loader
spec.loader.exec_module(module)


class EntryReadinessTests(unittest.TestCase):
    def test_repo_does_not_promote_owner_accepted_rt0_to_release_exit(self):
        result = module.status()
        self.assertEqual(result["schema"], "rt1-entry-readiness-0.1")
        self.assertTrue(result["owner_functional_r0_accepted"])
        self.assertTrue(result["rt1_release_spec_consistent"])
        self.assertFalse(result["formal_rt0_exit_verified"])
        self.assertFalse(result["rt1_full_implementation_authorized"])
        self.assertEqual(result["allowed_now"], ["BOUNDED_FEASIBILITY_SPIKE_ONLY"])

    def test_missing_acceptance_is_not_inferred(self):
        with tempfile.TemporaryDirectory() as location:
            result = module.status(Path(location))
            self.assertFalse(result["owner_functional_r0_accepted"])
            self.assertFalse(result["rt1_full_implementation_authorized"])
            self.assertFalse(result["rt1_release_spec_consistent"])
            self.assertEqual(result["allowed_now"], [])

    def test_rt1_owner_visitor_controls_wait_for_real_handler_bootstrap(self):
        root = Path(__file__).resolve().parents[2]
        html = (root / "crates/vpr-owner-lab/ui/rt1.html").read_text(encoding="utf-8")
        script = (root / "crates/vpr-owner-lab/ui/rt1.js").read_text(encoding="utf-8")
        self.assertIn('<script src="/rt1.js" defer></script>', html)
        self.assertEqual(html.count('<script src="/rt1.js" defer></script>'), 1)
        self.assertIn('id="status" data-rt1-ready="false"', html)
        for name in ("provision", "publish", "pause", "unpublish", "join", "unlock-audio"):
            self.assertIn(f'id="{name}" type="button" disabled', html)
            self.assertIn(f'el("{name}")', script)
        self.assertIn('status.dataset.rt1Ready = "true";', script)
        self.assertLess(
            script.index('window.addEventListener("pagehide"'),
            script.index('status.dataset.rt1Ready = "true";'),
        )
        self.assertEqual(script.count('status.dataset.rt1Ready = "true";'), 1)

    def test_rt1_real_rtp_canary_checks_ready_handlers_not_only_loading_text(self):
        root = Path(__file__).resolve().parents[2]
        source = (
            root / "crates/vpr-owner-lab/ui/e2e/rt1-local-canary.spec.ts"
        ).read_text(encoding="utf-8")
        self.assertIn('data-rt1-ready", "true"', source)
        self.assertIn('toContainText("Ожидает проверки владельцем"', source)
        self.assertIn('waitUntil: "commit"', source)
        self.assertNotIn('not.toHaveText("Загрузка…"', source)
        self.assertIn('Chromium must receive actual inbound audio RTP packets', source)
        self.assertIn('revoked visitor must reject a valid late SDP answer', source)

    def test_owner_record_alone_does_not_grant_rt1_entry(self):
        with tempfile.TemporaryDirectory() as location:
            root = Path(location)
            target = root / "docs/release-evidence/rt0/OWNER_FUNCTIONAL_ACCEPTANCE.md"
            target.parent.mkdir(parents=True)
            target.write_text(
                "**Owner decision: R0 ACCEPTED / CLOSED for the agreed functional demonstration.**",
                encoding="utf-8",
            )
            result = module.status(root)
            self.assertTrue(result["owner_functional_r0_accepted"])
            self.assertFalse(result["rt1_full_implementation_authorized"])


if __name__ == "__main__":
    unittest.main()
