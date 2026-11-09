"""Hermetic safety gates for the exact-tree historical branch retirement utility."""
from __future__ import annotations

import importlib.util
from pathlib import Path
from unittest import TestCase, mock

MODULE = Path(__file__).resolve().parents[2] / "scripts" / "retire_identical_branches.py"
spec = importlib.util.spec_from_file_location("retire_identical_branches", MODULE)
assert spec and spec.loader
retirement = importlib.util.module_from_spec(spec)
spec.loader.exec_module(retirement)


class BranchRetirementTests(TestCase):
    def test_different_tree_cannot_be_retired(self):
        with mock.patch.object(retirement, "tree", side_effect=lambda sha: sha):
            item = retirement.classify("main", "old", "other", set())
        self.assertEqual(item["decision"], "retain")
        self.assertEqual(item["reason"], "different_complete_tree")

    def test_exact_tree_only_is_eligible(self):
        with mock.patch.object(retirement, "tree", return_value="identical"):
            item = retirement.classify("main", "old", "oldhead", set())
        self.assertEqual(item["decision"], "eligible")

    def test_open_pull_request_never_eligible(self):
        with mock.patch.object(retirement, "tree", side_effect=AssertionError("must not inspect")):
            item = retirement.classify("main", "open", "head", {"open"})
        self.assertEqual(item["decision"], "retain")
        self.assertEqual(item["reason"], "open_pull_request")

    def test_changed_main_never_pushed(self):
        with mock.patch.object(retirement, "remote_main_head", return_value="new"):
            with mock.patch.object(retirement, "git") as push:
                self.assertEqual(retirement.retire({"name": "old", "sha": "head"}, "old"), "main_changed")
                push.assert_not_called()

    def test_moved_branch_never_pushed(self):
        with mock.patch.object(retirement, "remote_main_head", return_value="main"):
            with mock.patch.object(retirement, "remote_branches", return_value=[("old", "newhead")]):
                with mock.patch.object(retirement, "git") as push:
                    self.assertEqual(retirement.retire({"name": "old", "sha": "head"}, "main"), "branch_changed")
                    push.assert_not_called()

    def test_push_has_exact_ref_lease(self):
        process = mock.Mock(returncode=0)
        with mock.patch.object(retirement, "remote_main_head", return_value="main"):
            with mock.patch.object(retirement, "remote_branches", return_value=[("old", "head")]):
                with mock.patch.object(retirement, "git", return_value=process) as git:
                    self.assertEqual(retirement.retire({"name": "old", "sha": "head"}, "main"), "deleted")
        git.assert_called_once_with(
            "push", "--force-with-lease=refs/heads/old:head",
            "origin", ":refs/heads/old", check=False,
        )

    def test_remote_main_is_checked_before_each_deletion(self):
        process = mock.Mock(returncode=0)
        with mock.patch.object(retirement, "remote_main_head", side_effect=["main", "changed"]):
            with mock.patch.object(retirement, "remote_branches", return_value=[("old", "head")]):
                with mock.patch.object(retirement, "git", return_value=process) as push:
                    self.assertEqual(retirement.retire({"name": "old", "sha": "head"}, "main"), "deleted")
                    self.assertEqual(retirement.retire({"name": "old", "sha": "head"}, "main"), "main_changed")
                    push.assert_called_once()

    def test_audit_checkpoint_survives_later_verification_error(self):
        import json
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            branches = [("first", "sha1"), ("second", "sha2")]
            with mock.patch.object(retirement, "git"):
                with mock.patch.object(retirement, "main_head", return_value="main"):
                    with mock.patch.object(retirement, "remote_branches", return_value=branches):
                        with mock.patch.object(retirement, "classify", side_effect=lambda m, n, h, p: {"name": n, "sha": h, "decision": "eligible", "reason": "identical_complete_tree"}):
                            with mock.patch.object(retirement, "open_pr_heads", side_effect=[set(), set(), RuntimeError("api unavailable")]):
                                with mock.patch.object(retirement, "retire", return_value="deleted"):
                                    with self.assertRaisesRegex(RuntimeError, "api unavailable"):
                                        retirement.run(out, execute=True)
            audit = json.loads((out / "inventory-after.json").read_text())
            self.assertEqual(audit["branches"][0]["outcome"], "deleted")
            self.assertEqual(audit["branches"][1]["outcome"], "verification_failed")
            self.assertEqual(audit["branches"][1]["error_type"], "RuntimeError")

    def test_missing_remote_main_fails_closed(self):
        process = mock.Mock(stdout="")
        with mock.patch.object(retirement, "git", return_value=process):
            with self.assertRaisesRegex(RuntimeError, "cannot verify remote main"):
                retirement.remote_main_head()
