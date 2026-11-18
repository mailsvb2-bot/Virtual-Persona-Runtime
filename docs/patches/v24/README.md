# VPR RT1 Memory v24 — audited incremental patch

Base exact main SHA: `ff84464de8cbba5f30ed4aabc07d682500f61ac4`.

Fixes defects found by re-auditing v23:
1. Install the patch installer and its own regression suite into the checkout, not just the HTTP and database tests.
2. Ensure manual `/remember` is versioned and errors map to HTTP 400/403/409/503 instead of treating every database problem as authorization failure.
3. `NeedsConfirmation` is **409**, not HTTP 200 with `saved:false` that UI could report as success.
4. Browser save handler explicitly verifies `ok===true` and `saved===true` before claiming success.
5. Installer checks both Rust and JS original text, preflights every new path, rejects symlinks, and restores originals on a simulated failure to write the second source.

## Usage

```bash
python3 scripts/apply_v24.py /path/to/clean/repo       # preview
python3 scripts/apply_v24.py /path/to/clean/repo --apply
cd /path/to/clean/repo
python3 -m unittest discover -s tests/architecture -p 'test_v24_patch_installer.py' -v
python3 -m unittest discover -s tests/architecture -p 'test_rt1_memory_http_canonical.py' -v
cargo test --locked -p vpr-owner-lab --features rt1-memory-postgres
# only against disposable TLS PostgreSQL with migrations 0001..0005:
cargo test --locked -p vpr-rt1-text-memory-postgres -- --test-threads=1 --include-ignored
```

No public release promotion, main modification, or production migration performed. The PostgreSQL integration test is source only until run in CI against disposable PostgreSQL. Live browser avatar E2E and provider remote STOP remain unverified.
