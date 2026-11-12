#!/usr/bin/env python3
"""Reproducible, read-only, full-line Git inventory for every remote branch.

A Git history comparison is NOT proof that source lines are absent from main.
For each branch record both patch-equivalent unique commits and the complete
current file-content diff. This tool deliberately never merges or deletes refs.
"""
from __future__ import annotations

import argparse
import json
import os
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


# Block mapping is deliberately based on repository paths, not branch names:
# a single long-lived branch can contain several independent functional areas.
# Unknown paths remain visible instead of silently disappearing from the report.
BLOCK_PATHS = (
    ("rt1-media-worker", ("scripts/rt1_", "tests/e2e/test_rt1_")),
    ("rt1-owner-visitor", ("crates/vpr-owner-lab/src/rt1_", "crates/vpr-owner-lab/src/http_rt1", "crates/vpr-owner-lab/ui/rt1", "crates/vpr-owner-lab/ui/e2e/rt1")),
    ("rt0-voice-and-playback", ("crates/vpr-owner-lab/src/state/voice", "crates/vpr-owner-lab/src/http_voice", "crates/vpr-owner-lab/ui/src/", "crates/vpr-owner-lab/ui/dist/", "scripts/metrotrance", "tests/e2e/test_metrotrance")),
    ("providers-and-runtime", ("crates/vpr-provider-", "crates/vpr-runtime/", "crates/vpr-integration/", "crates/vpr-owner-lab/src/providers/")),
    ("build-and-security", (".github/", "Cargo.lock", "Cargo.toml", "package-lock.json", "crates/vpr-owner-lab/ui/package")),
    ("documentation-and-operations", ("docs/", "README", "scripts/audit_branch", "scripts/retire_")),
)


def functional_block(path: str) -> str:
    for block, prefixes in BLOCK_PATHS:
        if path.startswith(prefixes):
            return block
    return "other-integration"


def changed_paths(base: str, head: str) -> list[str]:
    """List BOTH rename endpoints, retaining tabs/newlines and Unicode.

    Rename detection would hide the removed source path and its subsystem;
    disabling it records source deletion and destination addition separately.
    """
    raw = subprocess.check_output([
        "git", "diff", "-z", "--name-only", "--no-renames", base, head, "--",
    ])
    return [os.fsdecode(path) for path in raw.split(bytes([0])) if path]


def branch_functional_blocks(base: str, head: str) -> dict[str, dict]:
    """Report introduced files per subsystem, never infer merge safety from history."""
    blocks: dict[str, dict] = {}
    for path in changed_paths(base, head):
        block = blocks.setdefault(functional_block(path), {
            "introduced_paths": [], "absent_from_main": [],
            "identical_to_main": [], "different_from_main": [],
        })
        block["introduced_paths"].append(path)
        main_blob = subprocess.run(
            ["git", "rev-parse", f"origin/main:{path}"],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False,
        )
        branch_blob = subprocess.run(
            ["git", "rev-parse", f"{head}:{path}"],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False,
        )
        if main_blob.returncode != 0 and branch_blob.returncode == 0:
            block["absent_from_main"].append(path)
        elif (main_blob.returncode == 0 and branch_blob.returncode == 0
              and main_blob.stdout == branch_blob.stdout
              and subprocess.run(
                  ["git", "diff", "--quiet", "--no-ext-diff", "--no-renames",
                   "origin/main", head, "--", f":(literal){path}"],
                  check=False,
              ).returncode == 0):
            # Matching blob IDs alone ignore executable-bit and symlink mode
            # differences; the Git diff also compares tree-entry metadata.
            block["identical_to_main"].append(path)
        else:
            # Even when a whole file differs, the useful hunks may already
            # exist in main. Require review, not a blind file overwrite.
            block["different_from_main"].append(path)
    return dict(sorted(blocks.items()))


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
        # A newly introduced path missing from main is a porting candidate,
        # not authorization to merge stale implementations automatically.
        # -z preserves literal paths (Unicode, tabs, newlines and whitespace).
        # Never parse Git's human-readable quoted filename output.
        raw_paths = subprocess.check_output([
            "git", "diff", "-z", "--name-only", "--diff-filter=A",
            base, branch, "--",
        ])
        introduced_paths = [
            os.fsdecode(path) for path in raw_paths.split(bytes([0])) if path
        ]
        missing_paths = [
            path for path in introduced_paths
            if subprocess.run(
                ["git", "cat-file", "-e", f"origin/main:{path}"],
                check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            ).returncode != 0
        ]
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
            "introduced_paths_missing_from_main": missing_paths,
            "introduced_patch_bytes": len(introduced),
            "behind": int(counts[0]),
            "ahead": int(counts[1]),
            "patch_unique_commits": unique,
            "changed_files": changed,
            "diff_bytes": len(patch),
            "full_diff": patch_name,
            "content_identical_to_main": not patch,
            "functional_blocks": branch_functional_blocks(base, branch),
        })
    # A block-first view of the same canonical audit, without a competing
    # inventory implementation or an automatic merge execution path.
    by_block: dict[str, list[dict]] = {}
    for item in summary["branches"]:
        for block, paths in item["functional_blocks"].items():
            by_block.setdefault(block, []).append({
                "branch": item["name"],
                "head_sha": item["head_sha"],
                "behind_main": item["behind"],
                "absent_from_main": paths["absent_from_main"],
                "different_from_main": paths["different_from_main"],
                "identical_to_main": paths["identical_to_main"],
                "introduced_paths": paths["introduced_paths"],
                "requires_manual_review": bool(paths["absent_from_main"] or paths["different_from_main"]),
            })
    (output / "functional-blocks.json").write_text(
        json.dumps({"main_sha": main, "blocks": dict(sorted(by_block.items()))},
                   indent=2, ensure_ascii=True) + "\n", encoding="utf-8"
    )
    (output / "summary.json").write_text(
        json.dumps(summary, indent=2, ensure_ascii=True) + "\n", encoding="utf-8"
    )
    (output / "README.txt").write_text(
        "Every .introduced.patch records the full branch-introduced changes "
        "from the common ancestor, including content already integrated in main. "
        "Every numbered .patch is the full line-by-line current-content diff from "
        "the pinned main SHA to the named branch, not an automatically safe merge. "
        "Missing introduced paths are porting candidates, not proof of safe code. "
        "Also review existing paths for missing behavior. Review authorship, tests "
        "and release gates before porting. functional-blocks.json groups all "
        "introduced changes by subsystem and marks path-level comparisons; "
        "it never equates different files or unique commit history with missing code.\n",
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
