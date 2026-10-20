"""Windows one-command voice launcher regression checks."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
ENTRY = ROOT / "START_VPR_WITH_VOICE.cmd"
LAUNCHER = ROOT / "scripts" / "windows-owner-lab-restart.ps1"

class VoiceLaunchContract(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.entry = ENTRY.read_text(encoding="utf-8").lower()
        cls.launcher = LAUNCHER.read_text(encoding="utf-8").lower()

    def test_default_is_piper(self):
        lines = [line.strip() for line in self.entry.splitlines()]
        self.assertTrue(any(line.startswith("powershell.exe") and line.endswith(" -piper") for line in lines))

    def test_sapi_requires_explicit_opt_in(self):
        self.assertIn('--system-voice', self.entry)
        self.assertNotIn('call "%~dp0scripts\\start-rt0-sapi.cmd"', self.entry)

    def test_fails_closed_on_errors(self):
        self.assertIn('exit /b %vpr_exit%', self.entry)
        self.assertIn('goto usage', self.entry)

    def test_canary_readiness_before_handoff(self):
        self.assertIn('assert-canaryreadiness', self.launcher)
        self.assertIn('wait-canarylabready', self.launcher)

if __name__ == "__main__":
    unittest.main()
