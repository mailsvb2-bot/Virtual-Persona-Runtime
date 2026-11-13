#!/usr/bin/env python3
"""Safely retire branches that are provably preserved in main.

No inference from cherry-pick counts, patch IDs, or filenames. A different
complete tree is eligible ONLY when its exact branch tip is a Git ancestor
of main. The remote ref is re-read before each lease-guarded push.
All branch decisions are exported before deletion for independent review.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
from pathlib import Path


def git(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", *args], check=check, text=True, encoding="utf-8",
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )


def remote_branches() -> list[tuple[str, str]]:
    rows = []
    for line in git("ls-remote", "--heads", "origin").stdout.splitlines():
        sha, ref = line.split("\t", 1)
        prefix = "refs/heads/"
        if ref.startswith(prefix) and ref != "refs/heads/main":
            rows.append((ref.removeprefix(prefix), sha))
    return sorted(rows)


def tree(sha: str) -> str:
    return git("rev-parse", f"{sha}^{{tree}}").stdout.strip()


def main_head() -> str:
    return git("rev-parse", "refs/remotes/origin/main").stdout.strip()


def remote_main_head() -> str:
    """Read authoritative remote main; fail closed on missing/ambiguous refs."""
    lines = git("ls-remote", "--heads", "origin", "refs/heads/main").stdout.splitlines()
    matches = [line.split("\t", 1)[0] for line in lines
               if line.endswith("\trefs/heads/main")]
    if len(matches) != 1 or len(matches[0]) != 40:
        raise RuntimeError("cannot verify remote main ref")
    return matches[0]


def exact_ancestor(head_sha: str, main_sha: str) -> bool:
    """Only a whole-history Git ancestry proof can authorize a differing tree.

    This cannot infer source equivalence from a cherry-pick, patch ID or diff.
    """
    result = git("merge-base", "--is-ancestor", head_sha, main_sha, check=False)
    if result.returncode not in (0, 1):
        raise RuntimeError("cannot verify Git ancestor relationship")
    return result.returncode == 0


def classify(main_sha: str, name: str, head_sha: str, protected: set[str]) -> dict:
    item = {"name": name, "sha": head_sha, "decision": "retain"}
    if name in protected:
        item["reason"] = "open_pull_request"
        return item
    try:
        item["tree_sha"] = tree(head_sha)
    except subprocess.CalledProcessError:
        item["reason"] = "unknown_commit"
        return item
    item["reason"] = "different_complete_tree"
    if item["tree_sha"] == tree(main_sha):
        item["decision"] = "eligible"
        item["reason"] = "identical_complete_tree"
    elif exact_ancestor(head_sha, main_sha):
        # Every reachable commit is already in current main history.
        # No extra code or historical commit can be lost by deleting its ref.
        item["decision"] = "eligible"
        item["reason"] = "exact_branch_tip_ancestor_of_main"
    return item


def open_pr_heads() -> set[str]:
    """Query every page; on API failure fail closed, never delete anything."""
    import urllib.request

    token = os.environ.get("GITHUB_TOKEN", "")
    repo = os.environ.get("GITHUB_REPOSITORY", "")
    if not token or not repo or "/" not in repo:
        raise RuntimeError("GITHUB_TOKEN and GITHUB_REPOSITORY required to delete branches")
    protected: set[str] = set()
    page = 1
    while True:
        req = urllib.request.Request(
            f"https://api.github.com/repos/{repo}/pulls?state=open&per_page=100&page={page}",
            headers={
                "Authorization": f"Bearer {token}",
                "Accept": "application/vnd.github+json",
                "X-GitHub-Api-Version": "2022-11-28",
            },
        )
        with urllib.request.urlopen(req, timeout=20) as response:
            batch = json.load(response)
        if not isinstance(batch, list):
            raise RuntimeError("unexpected GitHub PR response")
        for pr in batch:
            if pr.get("head", {}).get("repo", {}).get("full_name") == repo:
                protected.add(pr["head"]["ref"])
        if len(batch) < 100:
            return protected
        page += 1


def retire(item: dict, expected_main: str) -> str:
    """Force-with-lease prevents deleting a branch modified after classification."""
    name, sha = item["name"], item["sha"]
    if remote_main_head() != expected_main:
        return "main_changed"
    current = dict(remote_branches()).get(name)
    if current != sha:
        return "branch_changed"
    command = git(
        "push", f"--force-with-lease=refs/heads/{name}:{sha}",
        "origin", f":refs/heads/{name}", check=False,
    )
    return "deleted" if command.returncode == 0 else "delete_denied"


def run(output: Path, execute: bool) -> dict:
    output.mkdir(parents=True, exist_ok=True)
    # Unshallow checkout and fetch all remote heads before comparing full trees.
    git("fetch", "--no-tags", "--prune", "origin",
        "+refs/heads/*:refs/remotes/origin/*")
    main_sha = main_head()
    protected = open_pr_heads() if execute else set()
    branches = [classify(main_sha, name, sha, protected)
                for name, sha in remote_branches()]
    summary = {"main_sha": main_sha, "execute": execute, "branches": branches}
    # Write snapshot BEFORE any deletion.
    (output / "inventory-before.json").write_text(
        json.dumps(summary, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    def checkpoint() -> None:
        # Commit an audit checkpoint before the next potentially failing API call.
        staged = output / "inventory-after.json.tmp"
        staged.write_text(json.dumps(summary, indent=2, ensure_ascii=False) + "\n",
                          encoding="utf-8")
        staged.replace(output / "inventory-after.json")

    checkpoint()
    if execute:
        # Recheck PR protections for each deletion attempt; network failure
        # aborts rather than skipping an unknown protected ref.
        for item in branches:
            if item["decision"] != "eligible":
                continue
            try:
                if item["name"] in open_pr_heads():
                    item["outcome"] = "protected_by_new_pr"
                else:
                    item["outcome"] = retire(item, main_sha)
            except Exception as exc:
                item["outcome"] = "verification_failed"
                item["error_type"] = type(exc).__name__
                checkpoint()
                raise
            checkpoint()
    print(json.dumps({
        "main_sha": main_sha,
        "total": len(branches),
        "identical": sum(x["reason"] == "identical_complete_tree" for x in branches),
        "ancestor": sum(x["reason"] == "exact_branch_tip_ancestor_of_main" for x in branches),
        "deleted": sum(x.get("outcome") == "deleted" for x in branches),
        "different_tree": sum(x["reason"] == "different_complete_tree" for x in branches),
    }))
    return summary


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, default=Path("branch-retirement"))
    parser.add_argument("--execute", action="store_true")
    args = parser.parse_args()
    run(args.out, args.execute)


if __name__ == "__main__":
    main()
