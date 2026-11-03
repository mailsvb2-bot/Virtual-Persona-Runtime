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
        cls.commands = [
            line.strip()
            for line in cls.entry.splitlines()
            if line.strip().startswith("powershell.exe ")
        ]

    def test_default_is_piper_with_frozen_rt0_candidate(self):
        self.assertTrue(
            any(line.endswith(" -piper -rt0evidence") for line in self.commands),
            "Default launch must select Piper and strict exact-candidate RT0 evidence",
        )

    def test_male_piper_is_explicit_and_keeps_rt0_evidence(self):
        self.assertIn('--piper-denis', self.entry)
        self.assertTrue(
            any(
                line.endswith(" -piper -pipervoice ru_ru-denis-medium -rt0evidence")
                for line in self.commands
            ),
            "Explicit male Piper must retain strict RT0 candidate verification",
        )

    def test_no_voice_launch_can_silently_switch_to_main(self):
        self.assertTrue(self.commands, "At least one PowerShell launch must exist")
        for command in self.commands:
            with self.subTest(command=command):
                self.assertIn(" -rt0evidence", command)

    def test_sapi_requires_explicit_opt_in(self):
        self.assertIn('--system-voice', self.entry)
        self.assertNotIn('call "%~dp0scripts\\start-rt0-sapi.cmd"', self.entry)
        self.assertFalse(any(" -systemvoice" in line and " -piper" in line for line in self.commands))

    def test_fails_closed_on_errors(self):
        self.assertIn('exit /b %vpr_exit%', self.entry)
        self.assertIn('goto usage', self.entry)

    def test_canary_readiness_before_handoff(self):
        self.assertIn('assert-canaryreadiness', self.launcher)
        self.assertIn('wait-canarylabready', self.launcher)

    def test_strict_mode_keeps_candidate_without_switch_or_pull(self):
        self.assertIn("if (-not $rt0evidence)", self.launcher)
        self.assertIn("git switch main", self.launcher)
        self.assertIn("git pull --ff-only", self.launcher)
        self.assertIn("frozen rt0 candidate", self.launcher)


if __name__ == "__main__":
    unittest.main()
