import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[2] / "scripts/rt1_offline_persona_inventory.py"
spec = importlib.util.spec_from_file_location("rt1_inventory", SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class InventoryTests(unittest.TestCase):
    def classify(self, doc):
        with tempfile.TemporaryDirectory() as root:
            file = Path(root) / "persona.json"
            file.write_text(json.dumps(doc), encoding="utf-8")
            return module.inspect(file)

    def test_v2_requires_owner_review_and_reports_no_content(self):
        result = self.classify({
            "schema_version": "vpr-reviewed-owner-persona-2",
            "snapshot": {
                "persona_id": "sample-owner",
                "persona_version": 3,
                "claims": [{
                    "claim_id": "sample-claim",
                    "history_complete": True,
                    "revisions": [{"revision": 3, "statement": "example-content", "kind": "preference", "source": "owner", "verification": "owner_verified", "derivation": "direct"}],
                }],
            },
        })
        self.assertEqual(result["classification"], "OWNER_REVIEW_ONLY")
        self.assertFalse(result["automatic_migration_allowed"])
        self.assertNotIn("example-content", json.dumps(result))
        self.assertNotIn("sample-owner", json.dumps(result))

    def test_v1_migration_does_not_invent_history(self):
        result = self.classify({
            "schema_version": "vpr-reviewed-owner-persona-1",
            "snapshot": {
                "persona_id": "legacy",
                "persona_version": 4,
                "claims": [{"claim_id": "x", "revision": 4, "statement": "known", "kind": "factual"}],
            },
        })
        self.assertEqual(result["classification"], "OWNER_REVIEW_ONLY")
        self.assertIn("HISTORY_PARTIAL", result["reason"])

    def test_corrupt_revisions_are_rejected(self):
        base = {
            "schema_version": "vpr-reviewed-owner-persona-2",
            "snapshot": {"persona_id": "owner", "persona_version": 3,
                         "claims": [{"claim_id": "c", "history_complete": True,
                                     "revisions": [None]}]},
        }
        self.assertEqual(self.classify(base)["classification"], "REJECT")
        base["snapshot"]["claims"][0]["revisions"] = [{"revision": 2, "statement": "x"}]
        self.assertEqual(self.classify(base)["classification"], "REJECT")
        for bad_revision in (-1, 0, True, 10001):
            legacy = {"schema_version": "vpr-reviewed-owner-persona-1",
                      "snapshot": {"persona_id": "owner", "persona_version": 3,
                                   "claims": [{"claim_id": "c", "revision": bad_revision,
                                               "statement": "x", "kind": "factual"}]}}
            self.assertEqual(self.classify(legacy)["classification"], "REJECT")
        legacy["snapshot"]["claims"][0] = {"claim_id": "c", "revision": 2}
        self.assertEqual(self.classify(legacy)["classification"], "REJECT")

    def test_invalid_states_fail_closed(self):
        for document in ({}, {"schema_version": "unknown", "snapshot": {}}, {
            "schema_version": "vpr-reviewed-owner-persona-2",
            "snapshot": {"persona_id": "x", "persona_version": True, "claims": []},
        }):
            with self.subTest(document=document):
                self.assertEqual(self.classify(document)["classification"], "REJECT")


if __name__ == "__main__":
    unittest.main()
