#!/usr/bin/env python3
"""Reproducible, read-only, full-line Git inventory for every remote branch.

A Git history comparison is NOT proof that source lines are absent from main.
For each branch record both patch-equivalent unique commits and the complete
current file-content diff. This tool deliberately never merges or deletes refs.
"""
from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path


def git(*args: str) -> str:
    return subprocess.check_output(
        ["git", *args], text=True, encoding="utf-8", errors="replace"
    ).strip()


def branches() -> list[str]:
    names = git("for-each-ref", "--format=%(refname:short)", "refs/remotes/origin")
    return sorted(
        name for name in names.splitlines()
        if name.startswith("origin/") and name not in {"origin/HEAD", "origin/main"}
    )


def inventory(output: Path) -> dict:
    output.mkdir(parents=True, exist_ok=True)
    main = git("rev-parse", "refs/remotes/origin/main")
    summary = {"main_sha": main, "branches": []}
    for index, branch in enumerate(branches(), 1):
        name = branch.removeprefix("origin/")
        head = git("rev-parse", branch)
        counts = git("rev-list", "--left-right", "--count", f"origin/main...{branch}").split()
        unique = git(
            "log", "--format=%H", "--right-only", "--cherry-pick",
            f"origin/main...{branch}",
        ).splitlines()
        # Git diff captures every content hunk, including code with an old or
        # cherry-picked commit ID. Binary changes are included as Git patches.
        patch = subprocess.check_output(
            ["git", "diff", "--binary", "--no-ext-diff", "--find-renames",
             "--no-color", "origin/main", branch, "--"]
        )
        base = git("merge-base", "origin/main", branch)
        introduced = subprocess.check_output(
            ["git", "diff", "--binary", "--no-ext-diff", "--find-renames",
             "--no-color", base, branch, "--"]
        )
        introduced_name = f"{index:03d}.introduced.patch"
        (output / introduced_name).write_bytes(introduced)
        patch_name = f"{index:03d}.patch"
        (output / patch_name).write_bytes(patch)
        files = git("diff", "--name-status", "--find-renames", "origin/main", branch, "--")
        changed = [line for line in files.splitlines() if line]
        summary["branches"].append({
            "name": name,
            "head_sha": head,
            "merge_base_sha": base,
            "introduced_patch": introduced_name,
            "introduced_patch_bytes": len(introduced),
            "behind": int(counts[0]),
            "ahead": int(counts[1]),
            "patch_unique_commits": unique,
            "changed_files": changed,
            "diff_bytes": len(patch),
            "full_diff": patch_name,
            "content_identical_to_main": not patch,
        })
    (output / "summary.json").write_text(
        json.dumps(summary, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    (output / "README.txt").write_text(
        "Every .introduced.patch records the full branch-introduced changes "
        "from the common ancestor, including content already integrated in main. "
        "Every numbered .patch is the full line-by-line current-content diff from "
        "the pinned main SHA to the named branch, not an automatically safe merge. "
        "Review patch content, authorship, tests and release gates before porting.\n",
        encoding="utf-8",
    )
    return summary


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, default=Path("branch-audit"))
    args = parser.parse_args()
    result = inventory(args.out)
    print(json.dumps({
        "main_sha": result["main_sha"],
        "branches": len(result["branches"]),
        "nonidentical": sum(not x["content_identical_to_main"] for x in result["branches"]),
        "output": str(args.out),
    }, ensure_ascii=False))


if __name__ == "__main__":
    main()
