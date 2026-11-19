"""CPU-only wheelhouse provenance gate; no package installation or CUDA."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from rt1_musetalk_wheelhouse import SCHEMA, draft_manifest, verify_wheelhouse


class WheelhouseContract(unittest.TestCase):
    def fixture(self, base):
        root = Path(base) / "wheels"
        root.mkdir()
        name = "torch-2.0.1+cu118-cp310-cp310-linux_x86_64.whl"
        wheel = root / name
        with zipfile.ZipFile(wheel, "w") as handle:
            handle.writestr("torch-2.0.1.dist-info/WHEEL", "Wheel-Version: 1.0\\n")
            handle.writestr("torch-2.0.1.dist-info/METADATA", "Name: torch\\nVersion: 2.0.1\\n")
            handle.writestr("torch-2.0.1.dist-info/RECORD", "")
        data = wheel.read_bytes()
        manifest = Path(base) / "wheel-lock.json"
        manifest.write_text(json.dumps({
            "schema": SCHEMA, "python": "3.10",
            "wheels": [{"filename": name, "size": len(data),
                        "sha256": hashlib.sha256(data).hexdigest()}],
        }, sort_keys=True), encoding="utf-8")
        return root, manifest, hashlib.sha256(manifest.read_bytes()).hexdigest(), wheel

    def test_verified_wheelhouse_has_no_gpu_or_release_claim(self):
        with tempfile.TemporaryDirectory() as base:
            root, manifest, approved, wheel = self.fixture(base)
            result = verify_wheelhouse(root, manifest, approved)
            self.assertEqual(result["wheel_count"], 1)
            self.assertFalse(result["cuda_executed"])
            self.assertFalse(result["python_packages_installed"])
            self.assertFalse(result["production_release_approved"])

    def test_draft_manifest_requires_separate_approval_and_roundtrips(self):
        with tempfile.TemporaryDirectory() as base:
            root, _manifest, _approved, wheel = self.fixture(base)
            output = Path(base) / "candidate.json"
            candidate = draft_manifest(root, output)
            self.assertFalse(candidate["approved"])
            self.assertTrue(output.exists())
            self.assertEqual(
                candidate["draft_sha256_for_independent_review"],
                hashlib.sha256(output.read_bytes()).hexdigest(),
            )
            self.assertEqual(verify_wheelhouse(
                root, output, candidate["draft_sha256_for_independent_review"]
            )["wheel_count"], 1)
            with self.assertRaisesRegex(ValueError, "overwrite"):
                draft_manifest(root, output)

    def test_modified_wheel_and_unexpected_extra_file_fail_closed(self):
        with tempfile.TemporaryDirectory() as base:
            root, manifest, approved, wheel = self.fixture(base)
            wheel.write_bytes(wheel.read_bytes() + b"modified")
            with self.assertRaisesRegex(ValueError, "size or file kind"):
                verify_wheelhouse(root, manifest, approved)
        with tempfile.TemporaryDirectory() as base:
            root, manifest, approved, wheel = self.fixture(base)
            (root / "extra.txt").write_text("not in manifest")
            with self.assertRaisesRegex(ValueError, "unexpected or missing"):
                verify_wheelhouse(root, manifest, approved)

    def test_wrong_manifest_approval_and_duplicate_lock_entries_are_denied(self):
        with tempfile.TemporaryDirectory() as base:
            root, manifest, approved, wheel = self.fixture(base)
            with self.assertRaisesRegex(ValueError, "independent wheelhouse manifest SHA-256 approval required"):
                verify_wheelhouse(root, manifest, "")
            with self.assertRaisesRegex(ValueError, "differs from independent"):
                verify_wheelhouse(root, manifest, "0" * 64)
            data = json.loads(manifest.read_text())
            data["wheels"].append(dict(data["wheels"][0]))
            manifest.write_text(json.dumps(data))
            new_approval = hashlib.sha256(manifest.read_bytes()).hexdigest()
            with self.assertRaisesRegex(ValueError, "ambiguous or malformed"):
                verify_wheelhouse(root, manifest, new_approval)

    def test_unsafe_zip_member_paths_are_denied_even_with_approved_hash(self):
        for bad_name in ("../escape.py", "/absolute.py", "pkg/../../escape.py", "pkg\\\\evil.py"):
            with self.subTest(name=bad_name), tempfile.TemporaryDirectory() as base:
                root, manifest, approved, wheel = self.fixture(base)
                with zipfile.ZipFile(wheel, "a") as archive:
                    archive.writestr(bad_name, "bad")
                data = json.loads(manifest.read_text())
                data["wheels"][0]["sha256"] = hashlib.sha256(wheel.read_bytes()).hexdigest()
                data["wheels"][0]["size"] = wheel.stat().st_size
                manifest.write_text(json.dumps(data))
                digest = hashlib.sha256(manifest.read_bytes()).hexdigest()
                with self.assertRaisesRegex(ValueError, "unsafe wheelhouse ZIP member path"):
                    verify_wheelhouse(root, manifest, digest)

    def test_duplicate_or_missing_wheel_metadata_is_denied(self):
        import warnings
        with tempfile.TemporaryDirectory() as base:
            root, manifest, approved, wheel = self.fixture(base)
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", UserWarning)
                with zipfile.ZipFile(wheel, "a") as archive:
                    archive.writestr("torch-2.0.1.dist-info/WHEEL", "Wheel-Version: 1.0\\n")
            data = json.loads(manifest.read_text())
            data["wheels"][0]["sha256"] = hashlib.sha256(wheel.read_bytes()).hexdigest()
            data["wheels"][0]["size"] = wheel.stat().st_size
            manifest.write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError, "duplicate wheelhouse ZIP member"):
                verify_wheelhouse(root, manifest,
                                  hashlib.sha256(manifest.read_bytes()).hexdigest())
        with tempfile.TemporaryDirectory() as base:
            root, manifest, approved, wheel = self.fixture(base)
            with zipfile.ZipFile(wheel, "w") as archive:
                archive.writestr("pkg/module.py", "pass")
            data = json.loads(manifest.read_text())
            data["wheels"][0]["sha256"] = hashlib.sha256(wheel.read_bytes()).hexdigest()
            data["wheels"][0]["size"] = wheel.stat().st_size
            manifest.write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError, "WHEEL metadata missing"):
                verify_wheelhouse(root, manifest,
                                  hashlib.sha256(manifest.read_bytes()).hexdigest())

    def test_non_wheel_zip_bytes_are_denied(self):
        with tempfile.TemporaryDirectory() as base:
            root, manifest, approved, wheel = self.fixture(base)
            wheel.write_bytes(b"not-a-wheel")
            data = json.loads(manifest.read_text())
            data["wheels"][0]["sha256"] = hashlib.sha256(wheel.read_bytes()).hexdigest()
            data["wheels"][0]["size"] = wheel.stat().st_size
            manifest.write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError, "not a ZIP wheel"):
                verify_wheelhouse(root, manifest,
                                  hashlib.sha256(manifest.read_bytes()).hexdigest())

    def test_symlink_and_filename_traversal_fail_closed(self):
        with tempfile.TemporaryDirectory() as base:
            root, manifest, approved, wheel = self.fixture(base)
            replacement = root / "replacement.whl"
            replacement.write_bytes(wheel.read_bytes())
            wheel.unlink()
            try:
                wheel.symlink_to(replacement)
            except (OSError, NotImplementedError):
                self.skipTest("filesystem has no symlink support")
            with self.assertRaisesRegex(ValueError, "unexpected or missing|symlink"):
                verify_wheelhouse(root, manifest, approved)
        with tempfile.TemporaryDirectory() as base:
            root, manifest, approved, wheel = self.fixture(base)
            data = json.loads(manifest.read_text())
            data["wheels"][0]["filename"] = "../outside.whl"
            manifest.write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError, "ambiguous or malformed"):
                verify_wheelhouse(root, manifest,
                                  hashlib.sha256(manifest.read_bytes()).hexdigest())


if __name__ == "__main__":
    unittest.main()
