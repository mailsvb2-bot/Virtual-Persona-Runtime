"""Hermetic safety gates for exact-tree and proven-ancestor branch retirement."""
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
            with mock.patch.object(retirement, "exact_ancestor", return_value=False):
                with mock.patch.object(retirement, "exact_noop_merge", return_value=False):
                    item = retirement.classify("main", "old", "other", set())
        self.assertEqual(item["decision"], "retain")
        self.assertEqual(item["reason"], "different_complete_tree")

    def test_exact_tree_only_is_eligible(self):
        with mock.patch.object(retirement, "tree", return_value="identical"):
            item = retirement.classify("main", "old", "oldhead", set())
        self.assertEqual(item["decision"], "eligible")

    def test_different_tree_ancestor_is_safe_to_retire(self):
        with mock.patch.object(retirement, "tree", side_effect=lambda sha: sha):
            with mock.patch.object(retirement, "exact_ancestor", return_value=True):
                item = retirement.classify("main", "merged", "merged-head", set())
        self.assertEqual(item["decision"], "eligible")
        self.assertEqual(item["reason"], "exact_branch_tip_ancestor_of_main")

    def test_unique_differing_branch_is_never_eligible(self):
        with mock.patch.object(retirement, "tree", side_effect=lambda sha: sha):
            with mock.patch.object(retirement, "exact_ancestor", return_value=False):
                with mock.patch.object(retirement, "exact_noop_merge", return_value=False):
                    item = retirement.classify("main", "unique-code", "different-head", set())
        self.assertEqual(item["decision"], "retain")
        self.assertEqual(item["reason"], "different_complete_tree")

    def test_git_ancestor_indeterminate_fails_closed(self):
        result = mock.Mock(returncode=128)
        with mock.patch.object(retirement, "git", return_value=result):
            with self.assertRaisesRegex(RuntimeError, "cannot verify Git ancestor"):
                retirement.exact_ancestor("unknown-head", "main")

    def test_no_branch_with_open_pr_is_deleted_even_if_ancestor(self):
        with mock.patch.object(retirement, "tree", side_effect=AssertionError("no tree read")):
            with mock.patch.object(retirement, "exact_ancestor", side_effect=AssertionError("no check")):
                item = retirement.classify("main", "draft-pr", "ancestor-head", {"draft-pr"})
        self.assertEqual(item["decision"], "retain")
        self.assertEqual(item["reason"], "open_pull_request")

    def test_clean_noop_merge_is_eligible_for_retirement(self):
        with mock.patch.object(retirement, "tree", side_effect=lambda sha: sha):
            with mock.patch.object(retirement, "exact_ancestor", return_value=False):
                with mock.patch.object(retirement, "exact_noop_merge", return_value=True):
                    item = retirement.classify("main", "already-merged-code", "old-head", set())
        self.assertEqual(item["decision"], "eligible")
        self.assertEqual(item["reason"], "clean_merge_exactly_preserves_main_tree")

    def test_noop_merge_exact_tree_proof_not_just_clean_merge(self):
        with mock.patch.object(retirement, "tree", return_value="main-tree"):
            with mock.patch.object(retirement, "git", return_value=mock.Mock(
                returncode=0, stdout="different-tree",
            )):
                self.assertFalse(retirement.exact_noop_merge("main", "branch"))
        with mock.patch.object(retirement, "tree", return_value="main-tree"):
            with mock.patch.object(retirement, "git", return_value=mock.Mock(
                returncode=0, stdout="main-tree",
            )):
                self.assertTrue(retirement.exact_noop_merge("main", "branch"))

    def test_conflicting_and_failed_git_merge_retain_branch(self):
        for exit_code in (1, 2, 128):
            with self.subTest(exit_code=exit_code):
                with mock.patch.object(retirement, "git", return_value=mock.Mock(
                    returncode=exit_code, stdout="main-tree\\n",
                )):
                    with mock.patch.object(retirement, "tree", side_effect=AssertionError(
                        "no tree verification after failed merge",
                    )):
                        self.assertFalse(retirement.exact_noop_merge("main", "branch"))

    def test_open_pr_never_checks_merge_equivalence(self):
        with mock.patch.object(retirement, "exact_noop_merge", side_effect=AssertionError(
            "protected head cannot enter merge gate",
        )):
            item = retirement.classify("main", "draft-protected", "head", {"draft-protected"})
        self.assertEqual(item["decision"], "retain")

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

    def test_retirement_is_driven_by_successful_main_ci_not_arbitrary_push(self):
        workflow = (
            MODULE.parents[1] / ".github/workflows/retire-identical-branches.yml"
        ).read_text(encoding="utf-8")
        self.assertIn('workflows: ["CI"]', workflow)
        self.assertIn("types: [completed]", workflow)
        self.assertIn("branches: [main]", workflow)
        self.assertIn("github.event.workflow_run.conclusion == 'success'", workflow)
        self.assertIn("github.event.workflow_run.head_branch == 'main'", workflow)
        self.assertNotIn("  push:", workflow)

    def test_retirement_skips_stale_ci_sha_without_losing_retry_inventory(self):
        workflow = (
            MODULE.parents[1] / ".github/workflows/retire-identical-branches.yml"
        ).read_text(encoding="utf-8")
        self.assertIn("git ls-remote --heads origin refs/heads/main", workflow)
        self.assertIn("VERIFIED_CI_SHA", workflow)
        self.assertIn('run=false', workflow)
        self.assertIn("if: steps.current.outputs.run == 'true'", workflow)
        self.assertIn("if: always()", workflow)
        self.assertIn("branch-retirement-inventory", workflow)

    def test_missing_remote_main_fails_closed(self):
        process = mock.Mock(stdout="")
        with mock.patch.object(retirement, "git", return_value=process):
            with self.assertRaisesRegex(RuntimeError, "cannot verify remote main"):
                retirement.remote_main_head()
