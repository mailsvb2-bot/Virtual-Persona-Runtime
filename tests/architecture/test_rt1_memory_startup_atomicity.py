"""Lock RT1 memory initialization ordering without requiring PostgreSQL in CPU CI.

The runtime must not expose STORE when its sole maintenance worker failed to
spawn. This is a structural guard; real SQL tests cover database behavior.
"""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
MEMORY = ROOT / "crates/vpr-owner-lab/src/http_rt1_memory.rs"


class MemoryStartupAtomicity(unittest.TestCase):
    def test_store_is_published_only_after_worker_spawn_succeeds(self):
        source = MEMORY.read_text(encoding="utf-8")
        start = source.index("fn store() -> Result<Arc<TextMemory>, HttpResponse>")
        end = source.index("fn workspace_id()", start)
        body = source[start:end]
        spawn = body.index(".spawn(move ||")
        spawn_result = body.index('RT1_MEMORY_WORKER_UNAVAILABLE')
        publish = body.index("STORE\n        .set(Arc::clone(&db))")
        self.assertLess(spawn, spawn_result)
        self.assertLess(spawn_result, publish)
        self.assertIn('RT1_MEMORY_DB_UNAVAILABLE', body[publish:])


if __name__ == "__main__":
    unittest.main()
