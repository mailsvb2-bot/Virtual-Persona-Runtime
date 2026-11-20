"""Lock the public release gate into the existing canonical owner publish path.

RT1 local test publication must remain possible; external HTTPS + commercial
rights/budget cannot silently use the local in-memory Owner Lab rules.
"""
from pathlib import Path
import json
import unittest

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "crates/vpr-owner-lab/src"


class Rt1PublicReleaseGateTests(unittest.TestCase):
    def test_gate_precedes_expensive_provider_and_visitor_token_creation(self):
        http = (SRC / "http_rt1.rs").read_text(encoding="utf-8")
        publish = http.split("fn owner_publish(", 1)[1].split("fn owner_revoke(", 1)[0]
        self.assertIn("public_origin::publication_qualification()", publish)
        self.assertLess(
            publish.index("public_origin::publication_qualification()"),
            publish.index("state.engine.lock()"),
        )
        self.assertLess(
            publish.index("public_origin::publication_qualification()"),
            publish.index("Rt1OwnerController::new()"),
        )
        helper = (SRC / "http_rt1_public_origin.rs").read_text(encoding="utf-8")
        self.assertIn("pub(super) fn publication_qualification()", helper)
        self.assertIn('Err("RT1_PUBLIC_RELEASE_QUALIFICATION_REQUIRED")', helper)
        self.assertIn('let origin = configured().map_err', helper)
        self.assertIn('publication_qualification_for(origin.as_deref())', helper)

    def test_rights_and_budget_are_not_mislabeled_as_runtime(self):
        rights = json.loads((ROOT / "contracts/rt1-consent-rights-0.1.json").read_text())
        budget = json.loads((ROOT / "contracts/rt1-cost-budget-0.1.json").read_text())
        self.assertIs(rights["durable_consent_rights_state_implemented"], False)
        self.assertIs(budget["hard_session_budget_runtime_implemented"], False)
        self.assertIs(budget["prepublication_estimate_runtime_implemented"], False)
        mapper = (SRC / "http_rt1_start_errors.rs").read_text()
        self.assertIn("RT1_PUBLIC_RELEASE_QUALIFICATION_REQUIRED", mapper)


if __name__ == "__main__":
    unittest.main()
