"""Independent Owner Lab bootstrap recovery must never block the module graph."""
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HTML = ROOT / "crates/vpr-owner-lab/ui/index.html"
SERVER = ROOT / "crates/vpr-owner-lab/src/main.rs"
SCRIPT = ROOT / "crates/vpr-owner-lab/ui/bootstrap-watchdog.js"

class BootstrapWatchdogSafety(unittest.TestCase):
    def test_watchdog_registers_before_the_canonical_module(self):
        html = HTML.read_text(encoding="utf-8")
        watchdog = '<script defer src="/bootstrap-watchdog.js"></script>'
        entry = '<script type="module" src="/app.js"></script>'
        self.assertEqual(html.count(watchdog), 1)
        self.assertNotIn('<script async src="/bootstrap-watchdog.js"></script>', html)
        self.assertEqual(html.count(entry), 1)
        self.assertLess(html.index(watchdog), html.index(entry))
        self.assertIn('id="connect" class="primary" disabled', html)

    def test_bootstrap_does_not_speculatively_preload_entire_module_graph(self):
        html = HTML.read_text(encoding="utf-8")
        server = SERVER.read_text(encoding="utf-8")
        # The canonical entry owns its imports. Do not regress to speculative
        # modulepreload requests that starve the bounded local listener.
        self.assertNotIn('rel="modulepreload"', html)
        self.assertEqual(html.count('<script type="module" src="/app.js"></script>'), 1)
        self.assertEqual(html.count('<script type="module" src="/evidence-export.js"></script>'), 1)
        self.assertNotIn('<script type="module" src="/tts-selector.js"></script>', html)
        for path in ("/app.js", "/bootstrap-watchdog.js", "/evidence-export.js"):
            self.assertIn(f'"{path}" => static_response(', server)
        self.assertNotIn('<script async src="/mic-worklet.js">', html)
        self.assertNotIn('<script async src="/vendor/livekit-client.umd.js">', html)

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
