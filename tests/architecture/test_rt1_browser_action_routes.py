"""Fail if RT1 visitor/owner controls drift away from the canonical HTTP router.

This is a structural lock, not evidence of live provider, GPU or browser E2E.
"""
from html.parser import HTMLParser
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]
UI = ROOT / "crates/vpr-owner-lab/ui"
ROUTER = ROOT / "crates/vpr-owner-lab/src/http_rt1.rs"


class Buttons(HTMLParser):
    def __init__(self):
        super().__init__()
        self.identifiers = []

    def handle_starttag(self, tag, attrs):
        if tag == "button":
            self.identifiers.append(dict(attrs).get("id"))


class Rt1BrowserActions(unittest.TestCase):
    def test_every_rt1_button_has_a_registered_click_handler(self):
        markup = (UI / "rt1.html").read_text(encoding="utf-8")
        js = (UI / "rt1.js").read_text(encoding="utf-8")
        buttons = Buttons()
        buttons.feed(markup)
        self.assertEqual(len(buttons.identifiers), len(set(buttons.identifiers)))
        self.assertNotIn(None, buttons.identifiers)
        self.assertGreaterEqual(len(buttons.identifiers), 12)
        for identifier in buttons.identifiers:
            with self.subTest(button=identifier):
                direct = re.escape('el("' + identifier + '").addEventListener("click"')
                if identifier.startswith("memory-") and identifier != "memory-admin-refresh":
                    # All memory buttons register through the canonical action helper;
                    # verify the helper and every individual button ID.
                    self.assertIn('el(id).addEventListener("click"', js)
                    self.assertRegex(js, re.escape('memoryAction("' + identifier + '",'))
                else:
                    self.assertRegex(js, direct)

    def test_visitor_authority_precedes_provider_and_engine_construction(self):
        """Unauthorized START must not allocate provider/engine resources."""
        start = (ROOT / "crates/vpr-owner-lab/src/http_rt1_start.rs").read_text(
            encoding="utf-8"
        )
        manager = (ROOT / "crates/vpr-owner-lab/src/rt1_visitor_session_manager.rs").read_text(
            encoding="utf-8"
        )
        begin = start.index("pub(super) fn visitor_start(")
        end = start.index("\n}", begin)
        body = start[begin:end]
        authority = body.index(".preflight_visitor(&body.visitor_token)")
        reservation = body.index("reserve_session_identity(state)")
        provider = body.index("selected_provider_bundle(&session_id)")
        engine = body.index("engine_bootstrap::assemble(providers, true)")
        self.assertLess(authority, reservation)
        self.assertLess(reservation, provider)
        self.assertLess(provider, engine)
        self.assertIn("self.workspace.admit_visitor_turn(", manager)
        # Never remove the authoritative second admission or egress fence.
        self.assertIn("manager.open_with_fast_revoke(", (
            ROOT / "crates/vpr-owner-lab/src/rt1_visitor_conversation.rs"
        ).read_text(encoding="utf-8"))

    def test_aiortc_worker_uses_non_trickle_ice(self):
        """The canonical worker embeds ICE in SDP and denies trickle requests."""
        start = (ROOT / "crates/vpr-owner-lab/src/http_rt1_start.rs").read_text(
            encoding="utf-8"
        )
        signaling = (ROOT / "crates/vpr-owner-lab/src/rt1_visitor_signaling.rs").read_text(
            encoding="utf-8"
        )
        js = (UI / "rt1.js").read_text(encoding="utf-8")
        self.assertIn('"diagnostic_rtp": verified_worker::selected() ||', start)
        self.assertIn('if (start.diagnostic_rtp === false)', js)
        self.assertIn('await waitForIceGathering(connection)', js)
        self.assertIn('if self.remote.is_some() {', signaling)
        self.assertIn('return Err("RT1_VISITOR_ICE_DENIED".into());', signaling)

    def test_browser_api_paths_are_registered_by_single_canonical_router(self):
        js = (UI / "rt1.js").read_text(encoding="utf-8")
        router = ROUTER.read_text(encoding="utf-8")
        called = set(re.findall(r'["\'](/api/rt1/(?:owner|visitor)/[a-z-]+)["\']', js))
        self.assertGreaterEqual(len(called), 11)
        for path in called:
            with self.subTest(path=path):
                self.assertIn('"' + path + '"', router)
        self.assertNotIn("RT1_REAL_PROVIDER_FALLBACK", js)


if __name__ == "__main__":
    unittest.main()
