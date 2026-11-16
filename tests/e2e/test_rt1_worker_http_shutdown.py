"""A stalled HTTP request must not suppress the canonical media STOP drain."""
from __future__ import annotations

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import socket
import sys
import threading
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from rt1_media_worker_server import configure_worker_http_threads


class ServerShutdownTests(unittest.TestCase):
    def test_blocked_http_handler_does_not_block_control_server_close(self):
        entered = threading.Event()
        release = threading.Event()

        class BlockingHandler(BaseHTTPRequestHandler):
            def do_GET(self):
                entered.set()
                release.wait(5)
                try:
                    self.send_response(200)
                    self.end_headers()
                except OSError:
                    pass

            def log_message(self, *args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), BlockingHandler)
        configure_worker_http_threads(server)
        self.assertTrue(server.daemon_threads)
        self.assertFalse(server.block_on_close)
        serving = threading.Thread(target=server.serve_forever, daemon=True)
        serving.start()

        def stuck_request():
            try:
                with socket.create_connection(
                    ("127.0.0.1", server.server_port), timeout=2
                ) as sock:
                    sock.sendall(b"GET /blocked HTTP/1.1\r\nHost: localhost\r\n\r\n")
                    release.wait(5)
            except OSError:
                pass

        request = threading.Thread(target=stuck_request, daemon=True)
        request.start()
        try:
            self.assertTrue(entered.wait(2), "request never entered blocked handler")
            start = time.monotonic()
            server.shutdown()
            server.server_close()
            self.assertLess(
                time.monotonic() - start, 1.5,
                "control plane close was held by a blocked HTTP handler",
            )
            self.assertTrue(release.is_set() is False)
        finally:
            release.set()
            server.shutdown()
            server.server_close()
            serving.join(2)
            request.join(2)


if __name__ == "__main__":
    unittest.main()
