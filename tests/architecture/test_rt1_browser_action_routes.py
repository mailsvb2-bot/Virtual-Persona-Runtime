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
                pattern = re.escape('el("' + identifier + '").addEventListener("click"')
                self.assertRegex(js, pattern)

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
