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
