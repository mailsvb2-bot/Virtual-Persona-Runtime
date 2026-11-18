#!/usr/bin/env python3
"""Fail-closed, rollback-capable canonical RT1 memory-write patch for exact main SHA.

Preview by default; --apply requires a clean checkout and source match.
"""
from __future__ import annotations
import argparse
import difflib
import os
from pathlib import Path
import subprocess
import sys
import tempfile

EXPECTED_HEAD = 'ff84464de8cbba5f30ed4aabc07d682500f61ac4'
HTTP_PATH = Path('crates/vpr-owner-lab/src/http_rt1_memory.rs')
REGRESSION_PATH = Path('tests/architecture/test_rt1_memory_http_canonical.py')
INSTALLER_TEST_PATH = Path('tests/architecture/test_v24_patch_installer.py')
UI_PATH = Path('crates/vpr-owner-lab/ui/rt1.js')
PG_TEST_PATH = Path('staging/rt1-memory-postgres/tests/postgres_v24_manual_memory.rs')
OLD = '''            db.remember(
                principal,
                id,
                key,
                content,
                MemorySource::VisitorExplicit,
                365,
            )
            .map_err(denied)?;
            serde_json::json!({"ok":true,"saved":true})'''
NEW = '''            // HTTP and voice-outbox writes use one versioned PostgreSQL owner.
            let outcome = db.remember_reviewed(principal, id, key, content, 365)
                .map_err(|e| match e {
                    MemoryError::InvalidInput => error_response(400, "RT1_MEMORY_INVALID_INPUT"),
                    MemoryError::NotConsented => error_response(403, "RT1_MEMORY_DENIED"),
                    MemoryError::Conflict => error_response(409, "RT1_MEMORY_CONFLICT"),
                    MemoryError::Capacity => error_response(409, "RT1_MEMORY_CAPACITY"),
                    MemoryError::Unavailable => error_response(503, "RT1_MEMORY_DB_UNAVAILABLE"),
                })?;
            let label = match outcome {
                ConsolidationResult::Inserted => "inserted",
                ConsolidationResult::Corrected => "corrected",
                ConsolidationResult::Unchanged => "unchanged",
                ConsolidationResult::NeedsConfirmation => {
                    return Err(error_response(409, "RT1_MEMORY_CONFIRMATION_REQUIRED"));
                }
            };
            serde_json::json!({"ok":true,"saved":true,"outcome":label})'''
IMPORT_OLD = '    MemoryError, MemorySource, StablePrincipal, TextMemory,\n'
IMPORT_NEW = '    MemoryError, StablePrincipal, TextMemory,\n    advanced::ConsolidationResult,\n'

UI_OLD = '''    await api("/api/rt1/visitor/memory/remember", {
      ...requireMemorySession(), memory_token: memoryToken(), key, content, operation_id });
    memoryPrint("Сервер подтвердил сохранение заметки: " + key);'''
UI_NEW = '''    const result = await api("/api/rt1/visitor/memory/remember", {
      ...requireMemorySession(), memory_token: memoryToken(), key, content, operation_id });
    if (result.saved !== true || result.ok !== true) {
      throw new Error("Сохранение не подтверждено сервером.");
    }
    memoryPrint("Сервер подтвердил сохранение заметки: " + key);'''


def git(repo: Path, *args: str) -> str:
    return subprocess.check_output(['git', '-C', str(repo), *args], text=True, stderr=subprocess.PIPE).strip()


def patched(source: str) -> str:
    if source.count(OLD) != 1 or source.count(IMPORT_OLD) != 1:
        raise ValueError('Exact legacy route/import not found exactly once; refusing rewrite')
    out = source.replace(OLD, NEW, 1).replace(IMPORT_OLD, IMPORT_NEW, 1)
    if out.count('db.remember_reviewed(principal, id, key, content, 365)') != 1:
        raise ValueError('Patched route invariant failed')
    return out


def patched_ui(source: str) -> str:
    if source.count(UI_OLD) != 1:
        raise ValueError('Exact UI save handler not found exactly once')
    return source.replace(UI_OLD, UI_NEW, 1)


def atomic_write(path: Path, data: bytes) -> None:
    """Write adjacent temp file and replace. Caller handles multi-file rollback."""
    fd, temp = tempfile.mkstemp(prefix=f'.{path.name}.v24.', dir=path.parent)
    try:
        with os.fdopen(fd, 'wb') as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp, path)
    finally:
        if os.path.exists(temp):
            os.unlink(temp)


def apply_files(root: Path, sources: dict[Path, bytes], planned: dict[Path, bytes]) -> None:
    """Verify all paths before write and compensate on any write failure."""
    for rel, expected in sources.items():
        path = root / rel
        if path.is_symlink() or path.read_bytes() != expected:
            raise RuntimeError(f'Source changed or unsafe link: {rel}')
    for rel in planned:
        if (root / rel).exists() or (root / rel).is_symlink():
            raise RuntimeError(f'Destination already exists: {rel}')
    # Transform *before* changing filesystem so malformed source cannot create partial state.
    transformed = {
        HTTP_PATH: patched(sources[HTTP_PATH].decode('utf-8')).encode(),
        UI_PATH: patched_ui(sources[UI_PATH].decode('utf-8')).encode(),
    }
    created: list[Path] = []
    try:
        for rel, data in planned.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            atomic_write(path, data)
            created.append(path)
        for rel, data in transformed.items():
            atomic_write(root / rel, data)
    except BaseException:
        for rel, original in sources.items():
            if (root / rel).read_bytes() != original:
                atomic_write(root / rel, original)
        for path in reversed(created):
            path.unlink(missing_ok=True)
        raise


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('repo', type=Path)
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    root = args.repo.resolve()
    if git(root, 'rev-parse', 'HEAD') != EXPECTED_HEAD:
        raise SystemExit('HEAD mismatch; re-audit newer main before applying')
    if git(root, 'status', '--porcelain'):
        raise SystemExit('Checkout must be clean; refusing to overwrite any changes')
    sources = {rel: (root / rel).read_bytes() for rel in (HTTP_PATH, UI_PATH)}
    updated_http = patched(sources[HTTP_PATH].decode('utf-8'))
    updated_ui = patched_ui(sources[UI_PATH].decode('utf-8'))
    package = Path(__file__).resolve().parents[1]
    planned: dict[Path, bytes] = {}
    for rel in (REGRESSION_PATH, PG_TEST_PATH, INSTALLER_TEST_PATH, Path('scripts/apply_v24.py')):
        if (root / rel).exists() or (root / rel).is_symlink():
            raise SystemExit(f'Refusing to overwrite existing file {rel}')
        planned[rel] = (package / rel).read_bytes()
    for rel, new in ((HTTP_PATH, updated_http), (UI_PATH, updated_ui)):
        print(''.join(difflib.unified_diff(
            sources[rel].decode('utf-8').splitlines(True), new.splitlines(True),
            fromfile=str(rel), tofile=str(rel))))
    if args.apply:
        apply_files(root, sources, planned)
        print('Applied. Run full CI, feature Rust tests and isolated PostgreSQL tests before merge.')
    else:
        print('PREVIEW ONLY: nothing changed')
    return 0


if __name__ == '__main__':
    sys.exit(main())
