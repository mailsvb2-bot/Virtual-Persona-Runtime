"""Pinned MuseTalk GPU dependency probe (never imports CUDA)."""
import importlib.metadata
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from rt1_musetalk_environment import REQUIRED, audit, environment_snapshot, verify_approved_environment


def installed_exact(package):
    return REQUIRED.get(package, "1.10.0" if package == "aiortc" else "1.0")


class EnvironmentContract(unittest.TestCase):
    def test_exact_upstream_versions_pass_without_claiming_gpu(self):
        report = audit(python_version=(3, 10), installed=installed_exact)
        self.assertEqual(report["issues"], [])
        self.assertTrue(report["dependencies_match_upstream"])
        self.assertFalse(report["cuda_executed"])
        self.assertFalse(report["production_release_approved"])

    def test_python_312_not_mistaken_for_musetalk_python_310(self):
        report = audit(python_version=(3, 12), installed=installed_exact)
        self.assertFalse(report["dependencies_match_upstream"])
        self.assertIn("Python 3.10", report["issues"][0])

    def test_version_drift_and_missing_cuda_runtime_package_rejected(self):
        def packages(package):
            if package == "torch":
                return "2.4.1"
            if package == "av":
                raise importlib.metadata.PackageNotFoundError(package)
            return installed_exact(package)
        report = audit(python_version=(3, 10), installed=packages)
        self.assertTrue(any("torch: expected 2.0.1" in x for x in report["issues"]))
        self.assertTrue(any("av: not installed" in x for x in report["issues"]))

    def test_cuda_local_version_suffix_can_match_upstream_wheel(self):
        def packages(package):
            return "2.0.1+cu118" if package == "torch" else installed_exact(package)
        self.assertTrue(audit(python_version=(3, 10),
                              installed=packages)["dependencies_match_upstream"])

    def test_snapshot_deterministic_and_canonical_distribution_names(self):
        first = environment_snapshot(
            python_version=(3, 10),
            packages=[("Torch_Vision", "0.15.2"), ("torch", "2.0.1+cu118")],
        )
        second = environment_snapshot(
            python_version=(3, 10),
            packages=[("torch", "2.0.1+cu118"), ("torch-vision", "0.15.2")],
        )
        self.assertEqual(first, second)
        self.assertEqual(
            [p["name"] for p in first["packages"]], ["torch", "torch-vision"]
        )
        self.assertRegex(first["sha256"], r"^[0-9a-f]{64}$")
        self.assertEqual(first["python"], "3.10")

    def test_snapshot_rejects_ambiguous_or_empty_distribution_sets(self):
        for packages in ([], [("torch", "1"), ("Torch", "2")], [(None, "2")],
                         [("strange name", "1")], [("torch", "")]):
            with self.subTest(packages=packages), self.assertRaises(ValueError):
                environment_snapshot(python_version=(3, 10), packages=packages)

    def test_approved_snapshot_rejects_missing_and_drifted_pin(self):
        example = environment_snapshot(
            python_version=(3, 10), packages=[("torch", "2.0.1")]
        )
        with patch("rt1_musetalk_environment.audit",
                   return_value={"issues": []}), patch(
            "rt1_musetalk_environment.environment_snapshot",
            return_value=example,
        ), patch("rt1_musetalk_environment.subprocess.run") as pip_check:
            self.assertEqual(verify_approved_environment(example["sha256"]),
                             example["sha256"])
            pip_check.assert_called_once()
            with self.assertRaisesRegex(RuntimeError, "differs"):
                verify_approved_environment("0" * 64)
            with self.assertRaisesRegex(RuntimeError, "approval required"):
                verify_approved_environment(None)

    def test_broken_installed_dependency_graph_is_not_approved(self):
        import subprocess
        example = environment_snapshot(
            python_version=(3, 10), packages=[("torch", "2.0.1")]
        )
        with patch("rt1_musetalk_environment.audit",
                   return_value={"issues": []}), patch(
            "rt1_musetalk_environment.environment_snapshot",
            return_value=example,
        ), patch("rt1_musetalk_environment.subprocess.run",
                 side_effect=subprocess.CalledProcessError(1, ["pip", "check"])):
            with self.assertRaisesRegex(RuntimeError, "dependency graph"):
                verify_approved_environment(example["sha256"])

    def test_invalid_direct_dependencies_cannot_be_approved_by_snapshot(self):
        example = environment_snapshot(
            python_version=(3, 10), packages=[("torch", "2.0.1")]
        )
        with patch("rt1_musetalk_environment.audit",
                   return_value={"issues": ["torch: not approved"]}), patch(
            "rt1_musetalk_environment.environment_snapshot",
            return_value=example,
        ):
            with self.assertRaisesRegex(RuntimeError, "dependencies"):
                verify_approved_environment(example["sha256"])

    def test_unsupported_aiortc_release_rejected(self):
        def packages(package):
            return "2.0.0" if package == "aiortc" else installed_exact(package)
        self.assertTrue(any("aiortc: expected" in x for x in
                            audit(python_version=(3, 10), installed=packages)["issues"]))


if __name__ == "__main__":
    unittest.main()
