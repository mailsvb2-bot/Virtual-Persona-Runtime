"""Launch the canonical RT1 worker backed by actual aiortc RTP senders.

Bind loopback only; expose externally solely through authenticated TLS proxy.
An unresolved ledger at startup means remote senders may still exist: fail closed
instead of forging successful STOP or allocating a replacement session.
"""
from __future__ import annotations

import os
import sqlite3
from http.server import ThreadingHTTPServer

from rt1_aiortc_backend import AiortcSenderBackend
from rt1_media_worker_http import handler_for
from rt1_media_worker_lifecycle import MediaLifecycle


def main():
    token = os.environ["RT1_WORKER_TOKEN"]
    path = os.environ["RT1_WORKER_DB"]
    backend = AiortcSenderBackend()
    lifecycle = MediaLifecycle(path, backend)
    # Never trust process restart as proof that a remote RTP sender stopped.
    # Operations that need recovery require an explicit reconciliation policy.
    pending = lifecycle._db.execute(
        "SELECT id,phase FROM media_operations WHERE phase != 'stopped' LIMIT 1"
    ).fetchone()
    if pending is not None:
        backend.shutdown()
        raise RuntimeError(
            "Unresolved RTP allocation after restart; refuse worker admission: "
            + repr(pending)
        )
    server = ThreadingHTTPServer(
        ("127.0.0.1", int(os.environ.get("RT1_WORKER_PORT", "8099"))),
        handler_for(lifecycle, token),
    )
    try:
        server.serve_forever()
    finally:
        server.server_close()
        backend.shutdown()


if __name__ == "__main__":
    main()
