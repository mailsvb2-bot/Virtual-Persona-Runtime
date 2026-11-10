"""Branch integration inventory must preserve exact Git filenames."""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


AUDIT = Path(__file__).resolve().parents[2] / "scripts/audit_branch_diffs.py"


@unittest.skipIf(os.name == "nt", "Windows cannot represent LF in filenames")
class BranchInventoryFilenameTests(unittest.TestCase):
    def test_unicode_tab_and_newline_paths_are_not_git_quoted(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)

            def git(*args: str) -> None:
                subprocess.run(
                    ["git", *args], cwd=root, check=True,
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                )

            def add_file(name: str) -> None:
                (root / name).write_text("content\n", encoding="utf-8")

            git("init", "-q")
            git("config", "user.name", "VPR Test")
            git("config", "user.email", "vpr-test@example.invalid")
            add_file("base.py")
            git("add", ".")
            git("commit", "-qm", "base")
            git("branch", "-M", "main")
            git("checkout", "-qb", "feature")
            integrated = ["café.py", "tab\tname.py", "tail space .py"]
            absent = "newline\nname.py"
            for name in [*integrated, absent]:
                add_file(name)
            git("add", ".")
            git("commit", "-qm", "feature paths")
            git("update-ref", "refs/remotes/origin/feature", "HEAD")
            git("checkout", "-q", "main")
            for name in integrated:
                add_file(name)
            git("add", ".")
            git("commit", "-qm", "integrated paths")
            git("update-ref", "refs/remotes/origin/main", "HEAD")

            output = root / "branch-audit"
            subprocess.run(
                [sys.executable, str(AUDIT), "--out", str(output)],
                cwd=root, check=True, capture_output=True, text=True,
            )
            summary = json.loads((output / "summary.json").read_text(encoding="utf-8"))
            self.assertEqual(len(summary["branches"]), 1)
            self.assertEqual(summary["branches"][0]["name"], "feature")
            self.assertEqual(
                summary["branches"][0]["introduced_paths_missing_from_main"],
                [absent],
            )


if __name__ == "__main__":
    unittest.main()
