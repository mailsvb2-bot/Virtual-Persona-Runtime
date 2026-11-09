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
        with mock.patch.object(retirement, "main_head", return_value="new"):
            with mock.patch.object(retirement, "git") as push:
                self.assertEqual(retirement.retire({"name": "old", "sha": "head"}, "old"), "main_changed")
                push.assert_not_called()

    def test_moved_branch_never_pushed(self):
        with mock.patch.object(retirement, "main_head", return_value="main"):
            with mock.patch.object(retirement, "remote_branches", return_value=[("old", "newhead")]):
                with mock.patch.object(retirement, "git") as push:
                    self.assertEqual(retirement.retire({"name": "old", "sha": "head"}, "main"), "branch_changed")
                    push.assert_not_called()

    def test_push_has_exact_ref_lease(self):
        process = mock.Mock(returncode=0)
        with mock.patch.object(retirement, "main_head", return_value="main"):
            with mock.patch.object(retirement, "remote_branches", return_value=[("old", "head")]):
                with mock.patch.object(retirement, "git", return_value=process) as git:
                    self.assertEqual(retirement.retire({"name": "old", "sha": "head"}, "main"), "deleted")
        git.assert_called_once_with(
            "push", "--force-with-lease=refs/heads/old:head",
            "origin", ":refs/heads/old", check=False,
        )
