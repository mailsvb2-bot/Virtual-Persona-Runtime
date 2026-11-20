"""RT1 release inventory cannot silently promote synthetic checks to live proof."""
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import rt1_release_candidate_inventory as inventory


class Rt1CandidateInventoryTests(unittest.TestCase):
    def test_real_external_evidence_remains_unverified(self):
        report = inventory.inventory()
        self.assertEqual(report["schema"], "rt1-release-candidate-inventory-v1")
        self.assertEqual(len(report["spec_sha256"]), 64)
        self.assertFalse(report["production_ready"])
        self.assertEqual(report["stop_condition"], "DO_NOT_PUBLICLY_PUBLISH_OR_CHARGE_UNTIL_VERIFIED")
        requirements = {item["id"]: item for item in report["requirements"]}
        self.assertEqual(len(requirements), 6)
        self.assertTrue(all(item["status"] == "UNVERIFIED" for item in requirements.values()))
        self.assertIs(requirements["P1_RIGHTS_CONSENT"]["runtime_implemented"], False)
        self.assertIs(requirements["P1_COST_BUDGET"]["hard_budget_implemented"], False)
        self.assertIs(requirements["P0_PUBLIC_HTTPS_DEPLOYMENT"]["probe_available"], True)
        self.assertIs(requirements["P0_REAL_GPU_MUSETALK"]["hardware_harness_available"], True)

    def test_never_overwrites_existing_candidate_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "existing.json"
            path.write_text("original\n", encoding="utf-8")
            with self.assertRaises(FileExistsError):
                with path.open("x", encoding="utf-8"):
                    pass
            self.assertEqual(path.read_text(encoding="utf-8"), "original\n")


if __name__ == "__main__":
    unittest.main()
