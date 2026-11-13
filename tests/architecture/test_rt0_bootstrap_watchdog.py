"""Independent Owner Lab bootstrap recovery must never block the module graph."""
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HTML = ROOT / "crates/vpr-owner-lab/ui/index.html"
SERVER = ROOT / "crates/vpr-owner-lab/src/main.rs"
SCRIPT = ROOT / "crates/vpr-owner-lab/ui/bootstrap-watchdog.js"

class BootstrapWatchdogSafety(unittest.TestCase):
    def test_watchdog_loads_asynchronously_and_does_not_block_app(self):
        html = HTML.read_text(encoding="utf-8")
        self.assertIn('<script async src="/bootstrap-watchdog.js"></script>', html)
        self.assertNotIn('<script defer src="/bootstrap-watchdog.js"></script>', html)
        self.assertIn('<script type="module" src="/app.js"></script>', html)
        self.assertIn('id="connect" class="primary" disabled', html)
    def test_critical_owner_modules_are_preloaded_before_browser_body(self):
        html = HTML.read_text(encoding="utf-8")
        server = SERVER.read_text(encoding="utf-8")
        for module in ("owner-capture.js", "reference-capture.js"):
            preload = f'<link rel="modulepreload" href="/{module}">'
            self.assertEqual(html.count(preload), 1)
            self.assertLess(html.index(preload), html.index("</head>"))
            self.assertIn(f'"/{module}" => static_response(', server)
            self.assertNotIn(f'<script async src="/{module}">', html)
        self.assertIn('<script type="module" src="/app.js"></script>', html)
        # A preload must never execute a second application bootstrap.
        self.assertEqual(html.count('<script type="module" src="/app.js"></script>'), 1)

    def test_recovery_uses_static_local_code_only(self):
        server = SERVER.read_text(encoding="utf-8")
        script = SCRIPT.read_text(encoding="utf-8")
        self.assertIn('"/bootstrap-watchdog.js" => static_response(BOOTSTRAP_WATCHDOG_JS, JS_MIME)',server)
        self.assertIn('"Загрузка…"', script)
        self.assertIn('window.location.reload()',script)
        self.assertIn('status.dataset.bootstrapFailure = failureKind', script)
        self.assertIn('failureKind = "module-load"', script)
        self.assertIn('failureKind = "bootstrap-unconfirmed"', script)
        self.assertIn('failureKind = "module-execution"', script)
        self.assertIn('url.origin === window.location.origin', script)
        self.assertNotIn('console.log', script)
        self.assertNotIn('fetch(',script)
        self.assertNotIn('Authorization',script)

if __name__ == "__main__":
    unittest.main()
