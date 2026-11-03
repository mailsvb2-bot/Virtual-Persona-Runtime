"""Launch the canonical RT1 worker backed by actual aiortc RTP senders.

Bind loopback only; expose externally solely through authenticated TLS proxy.
An unresolved ledger at startup means remote senders may still exist: fail closed
instead of forging successful STOP or allocating a replacement session.
"""
from __future__ import annotations

import os
import logging
import threading
from http.server import ThreadingHTTPServer

from rt1_aiortc_backend import AiortcSenderBackend
from rt1_media_worker_http import handler_for
from rt1_media_worker_lifecycle import MediaLifecycle


LOG = logging.getLogger("rt1.worker")


def reconcile_until_shutdown(lifecycle: MediaLifecycle, shutdown: threading.Event, interval: float = 1.0):
    """Retry only durable unresolved STOPs, never CREATE or unverified receipts.

    An unreachable sender remains *uncertain* in SQLite. The control plane
    continues to reject a new allocation while it is uncertain. A runtime
    restart still fails closed when an old sender cannot be verified stopped.
    """
    if interval <= 0:
        raise ValueError("reconcile interval must be positive")
    while not shutdown.is_set():
        try:
            lifecycle.reconcile()
        except Exception:
            LOG.exception("RT1 STOP reconciliation pending; media remains fenced")
        shutdown.wait(interval)


def main():
    token = os.environ["RT1_WORKER_TOKEN"]
    path = os.environ["RT1_WORKER_DB"]
    backend = AiortcSenderBackend()
    lifecycle = MediaLifecycle(path, backend)
    # Restart is not a remote STOP receipt. Fence all inherited sessions
    # before exposing HTTP so an idempotent PUT cannot return a stale active
    # session. The existing reconciler attempts authenticated STOP where a
    # recoverable sender exists; otherwise SQLite remains uncertain and
    # admission stays denied without trapping the worker in a restart loop.
    lifecycle.quarantine_after_restart()
    server = ThreadingHTTPServer(
        ("127.0.0.1", int(os.environ.get("RT1_WORKER_PORT", "8099"))),
        handler_for(lifecycle, token),
    )
    shutdown = threading.Event()
    reconciler = threading.Thread(
        target=reconcile_until_shutdown,
        args=(lifecycle, shutdown),
        name="rt1-stop-reconcile",
        daemon=True,
    )
    reconciler.start()
    try:
        server.serve_forever()
    finally:
        server.server_close()
        shutdown.set()
        reconciler.join(timeout=3)
        backend.shutdown()


if __name__ == "__main__":
    main()
