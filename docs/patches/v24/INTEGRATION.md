# Integration and verification

This patch is designed for an **exact** clean `main` (`ff84464`). Run the installer preview and apply in a disposable worktree; do not use production databases. Patch installer copies four new files (its own installer, unit suite, architecture guard and PostgreSQL regression), and modifies HTTP/JS existing files. After applying, run all Rust, frontend, architecture, PostgreSQL CI and browser E2E. PR is subject to existing RT1 research and optional feature gates. The patch explicitly does not prove provider STOP or real GPU frame production.
