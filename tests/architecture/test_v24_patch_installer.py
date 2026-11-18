"""In-process tests for the patch transformer and transactional installer."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[2] / 'scripts' / 'apply_v24.py'
spec = importlib.util.spec_from_file_location('v24installer', SCRIPT)
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class PatchSafety(unittest.TestCase):
    def test_exact_route_rewritten_once(self):
        source = installer.IMPORT_OLD + installer.OLD
        output = installer.patched(source)
        self.assertNotIn('db.remember(', output)
        self.assertIn('ConsolidationResult::Corrected', output)
        self.assertIn('RT1_MEMORY_CONFIRMATION_REQUIRED', output)
        self.assertEqual(output.count('db.remember_reviewed('), 1)

    def test_rejects_modified_or_missing_route(self):
        for value in ('', installer.IMPORT_OLD + installer.OLD * 2):
            with self.assertRaises(ValueError):
                installer.patched(value)

    def fixture(self, root):
        http = root / installer.HTTP_PATH
        ui = root / installer.UI_PATH
        http.parent.mkdir(parents=True)
        ui.parent.mkdir(parents=True)
        original = (installer.IMPORT_OLD + installer.OLD).encode()
        html = installer.UI_OLD.encode()
        http.write_bytes(original)
        ui.write_bytes(html)
        return {installer.HTTP_PATH: original, installer.UI_PATH: html}

    def test_patch_ui_denies_false_success(self):
        result = installer.patched_ui(installer.UI_OLD)
        self.assertIn('result.saved !== true', result)
        self.assertIn('result.ok !== true', result)
        with self.assertRaises(ValueError):
            installer.patched_ui('missing')

    def test_duplicate_destination_has_no_side_effects(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            originals = self.fixture(root)
            destination = root / installer.PG_TEST_PATH
            destination.parent.mkdir(parents=True)
            destination.write_bytes(b'previous')
            with self.assertRaises(RuntimeError):
                installer.apply_files(root, originals, {installer.PG_TEST_PATH: b'different'})
            for rel, data in originals.items():
                self.assertEqual((root / rel).read_bytes(), data)
            self.assertEqual(destination.read_bytes(), b'previous')

    def test_atomic_multi_file_success(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            originals = self.fixture(root)
            installer.apply_files(root, originals, {installer.PG_TEST_PATH: b'fixture'})
            self.assertIn(b'remember_reviewed', (root / installer.HTTP_PATH).read_bytes())
            self.assertIn(b'result.saved !== true', (root / installer.UI_PATH).read_bytes())
            self.assertEqual((root / installer.PG_TEST_PATH).read_bytes(), b'fixture')

    def test_failure_rolls_back_created_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            originals = self.fixture(root)
            originals[installer.HTTP_PATH] = b'source missing legacy segment'
            (root / installer.HTTP_PATH).write_bytes(originals[installer.HTTP_PATH])
            with self.assertRaises(ValueError):
                installer.apply_files(root, originals, {installer.PG_TEST_PATH: b'fixture'})
            for rel, data in originals.items():
                self.assertEqual((root / rel).read_bytes(), data)
            self.assertFalse((root / installer.PG_TEST_PATH).exists())

    def test_second_write_failure_rolls_back_both_files(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            originals = self.fixture(root)
            real_write = installer.atomic_write
            failing = root / installer.UI_PATH
            def fail_once(path, data):
                if path == failing and data != originals[installer.UI_PATH]:
                    raise OSError('simulated UI failure')
                return real_write(path, data)
            with patch.object(installer, 'atomic_write', side_effect=fail_once):
                with self.assertRaises(OSError):
                    installer.apply_files(root, originals, {installer.PG_TEST_PATH: b'fixture'})
            for rel, data in originals.items():
                self.assertEqual((root / rel).read_bytes(), data)
            self.assertFalse((root / installer.PG_TEST_PATH).exists())

    def test_symlink_destination_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            originals = self.fixture(root)
            target = root / installer.PG_TEST_PATH
            target.parent.mkdir(parents=True)
            target.symlink_to(root / installer.HTTP_PATH)
            with self.assertRaises(RuntimeError):
                installer.apply_files(root, originals, {installer.PG_TEST_PATH: b'x'})


if __name__ == '__main__':
    unittest.main()
