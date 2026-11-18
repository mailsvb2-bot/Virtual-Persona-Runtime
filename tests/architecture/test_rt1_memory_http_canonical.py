"""Source-level guard: the UI HTTP write path may not bypass the versioned owner."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]
HTTP = ROOT / 'crates/vpr-owner-lab/src/http_rt1_memory.rs'
VOICE = ROOT / 'crates/vpr-owner-lab/src/rt1_memory_port.rs'
ADVANCED = ROOT / 'staging/rt1-memory-postgres/src/advanced.rs'


class CanonicalMemoryWrites(unittest.TestCase):
    def test_http_button_uses_reviewed_history(self):
        source = HTTP.read_text(encoding='utf-8')
        start = source.index('"/api/rt1/visitor/memory/remember" => {')
        end = source.index('"/api/rt1/visitor/memory/contradictions" => {', start)
        route = source[start:end]
        self.assertIn('db.remember_reviewed(principal, id, key, content, 365)', route)
        self.assertIn('ConsolidationResult::Corrected', route)
        self.assertIn('ConsolidationResult::Unchanged', route)
        self.assertNotRegex(route, r'\bdb\.remember\(')
        self.assertIn('MemoryError::Conflict => error_response(409', route)
        self.assertIn('"saved":true', route)
        self.assertIn('RT1_MEMORY_CONFIRMATION_REQUIRED', route)

    def test_ui_fails_closed_on_unsaved_response(self):
        ui = (ROOT / 'crates/vpr-owner-lab/ui/rt1.js').read_text(encoding='utf-8')
        start = ui.index('memoryAction("memory-save"')
        end = ui.index('memoryAction("memory-list"', start)
        self.assertIn('result.saved !== true || result.ok !== true', ui[start:end])

    def test_voice_and_http_share_versioned_owner(self):
        self.assertIn('.enqueue_explicit_fact(', VOICE.read_text(encoding='utf-8'))
        advanced = ADVANCED.read_text(encoding='utf-8')
        self.assertIn('remember_reviewed_in_grant', advanced)
        self.assertIn('vpr_memory.fact_versions', advanced)
        self.assertIn('vpr_memory.operation_receipts', advanced)


if __name__ == '__main__':
    unittest.main()
