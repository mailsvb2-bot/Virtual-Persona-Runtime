"""Pinned MuseTalk GPU dependency probe (never imports CUDA)."""
import importlib.metadata
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from rt1_musetalk_environment import REQUIRED, audit


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

    def test_unsupported_aiortc_release_rejected(self):
        def packages(package):
            return "2.0.0" if package == "aiortc" else installed_exact(package)
        self.assertTrue(any("aiortc: expected" in x for x in
                            audit(python_version=(3, 10), installed=packages)["issues"]))


if __name__ == "__main__":
    unittest.main()
