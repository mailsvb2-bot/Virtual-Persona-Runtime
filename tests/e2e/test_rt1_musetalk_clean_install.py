"""CPU-only offline installer tests: all subprocess effects are intercepted."""
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from rt1_musetalk_clean_install import (
    check_python310, install_approved, isolated_install_environment,
    locked_requirements, main,
)
from rt1_musetalk_wheelhouse import SCHEMA


class OfflineInstallContract(unittest.TestCase):
    def fixture(self, root):
        wheels = root / "wheelhouse"
        wheels.mkdir()
        filename = "demo_pkg-1.0-py3-none-any.whl"
        wheel = wheels / filename
        with zipfile.ZipFile(wheel, "w") as archive:
            archive.writestr("demo_pkg-1.0.dist-info/WHEEL", "Wheel-Version: 1.0\n")
            archive.writestr("demo_pkg-1.0.dist-info/METADATA",
                             "Name: demo-pkg\nVersion: 1.0\n")
            archive.writestr("demo_pkg-1.0.dist-info/RECORD", "")
        digest = hashlib.sha256(wheel.read_bytes()).hexdigest()
        manifest = root / "reviewed-lock.json"
        manifest.write_text(json.dumps({
            "schema": SCHEMA, "python": "3.10", "wheels": [
                {"filename": filename, "size": wheel.stat().st_size,
                 "sha256": digest}
            ],
        }), encoding="utf-8")
        return wheels, manifest, hashlib.sha256(manifest.read_bytes()).hexdigest(), digest

    def test_cpu_only_plan_has_exact_hash_and_performs_no_subprocess(self):
        with tempfile.TemporaryDirectory() as tmp:
            wheels, lock, approved, digest = self.fixture(Path(tmp))
            evidence, requirements = locked_requirements(wheels, lock, approved)
            self.assertEqual(evidence["wheel_count"], 1)
            self.assertIn("file://", requirements)
            self.assertIn("--hash=sha256:" + digest, requirements)
            with patch("rt1_musetalk_clean_install.subprocess.run") as runner:
                status = main([
                    "--wheelhouse", str(wheels), "--manifest", str(lock),
                    "--approved-manifest-sha256", approved,
                ])
            self.assertEqual(status, 0)
            runner.assert_not_called()

    def test_unapproved_and_mutated_wheelhouse_cannot_be_planned(self):
        with tempfile.TemporaryDirectory() as tmp:
            wheels, lock, approved, digest = self.fixture(Path(tmp))
            with self.assertRaises(ValueError):
                locked_requirements(wheels, lock, "0" * 64)
            wheel = next(wheels.iterdir())
            wheel.write_bytes(wheel.read_bytes() + b"tampering")
            with self.assertRaises(ValueError):
                locked_requirements(wheels, lock, approved)

    def test_installer_rejects_existing_destination_before_effects(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            wheels, lock, approved, digest = self.fixture(root)
            destination = root / "already-here"
            destination.mkdir()
            with patch("rt1_musetalk_clean_install.check_python310"), patch(
                "rt1_musetalk_clean_install.subprocess.run"
            ) as runner:
                with self.assertRaisesRegex(ValueError, "nonexistent"):
                    install_approved(wheels, lock, approved, Path(sys.executable), destination)
            runner.assert_not_called()

    def test_interpreter_must_report_exact_python_310(self):
        with tempfile.TemporaryDirectory() as tmp:
            interpreter = Path(sys.executable).resolve()
            with patch("rt1_musetalk_clean_install.subprocess.run",
                       return_value=SimpleNamespace(stdout="3.12\n")):
                with self.assertRaisesRegex(ValueError, "Python 3.10"):
                    check_python310(interpreter)
            with patch("rt1_musetalk_clean_install.subprocess.run",
                       return_value=SimpleNamespace(stdout="3.10\n")) as runner:
                check_python310(interpreter)
            self.assertIn("-I", runner.call_args.args[0])

    def test_opt_in_clean_install_uses_only_hash_locked_offline_pip(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            wheels, lock, approved, digest = self.fixture(root)
            destination = root / "new-private-venv"
            commands = []
            environments = []
            requirements_seen = []

            def capture(command, **kwargs):
                commands.append(command)
                environments.append(kwargs.get("env"))
                if "--require-hashes" in command:
                    requirements_seen.append(
                        Path(command[command.index("-r") + 1]).read_text()
                    )
                if "--emit-snapshot" in command:
                    return SimpleNamespace(stdout=json.dumps({
                        "issues": [],
                        "environment_snapshot": {"sha256": "a" * 64},
                    }))
                return SimpleNamespace(stdout="")

            with patch.dict(os.environ, {
                "PIP_TARGET": str(root / "forbidden-external-destination"),
                "PIP_PREFIX": str(root / "wrong-prefix"),
                "PIP_USER": "true",
                "PYTHONPATH": str(root / "untrusted-pythonpath"),
                "VIRTUAL_ENV": str(root / "other-venv"),
            }), patch("rt1_musetalk_clean_install.check_python310"), patch(
                "rt1_musetalk_clean_install.subprocess.run", side_effect=capture
            ):
                result = install_approved(wheels, lock, approved,
                                          Path(sys.executable), destination)
            self.assertTrue(destination.is_dir())
            self.assertFalse(result["installed_environment_approved"])
            self.assertFalse(result["gpu_executed"])
            self.assertFalse(result["production_release_approved"])
            self.assertEqual(result["installed_environment_sha256"], "a" * 64)
            self.assertEqual(len(requirements_seen), 1)
            self.assertIn("--hash=sha256:" + digest, requirements_seen[0])
            pip = next(c for c in commands if "install" in c)
            self.assertIn("--no-index", pip)
            self.assertIn("--no-deps", pip)
            self.assertIn("--require-hashes", pip)
            self.assertIn("--no-cache-dir", pip)
            self.assertTrue(any(c[-2:] == ["pip", "check"] for c in commands))
            for env in environments:
                self.assertEqual(env["PIP_CONFIG_FILE"], os.devnull)
                self.assertEqual(env["PYTHONNOUSERSITE"], "1")
                self.assertNotIn("PIP_TARGET", env)
                self.assertNotIn("PIP_PREFIX", env)
                self.assertNotIn("PIP_USER", env)
                self.assertNotIn("PYTHONPATH", env)
                self.assertNotIn("VIRTUAL_ENV", env)
            self.assertFalse((root / "forbidden-external-destination").exists())


if __name__ == "__main__":
    unittest.main()
