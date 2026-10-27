"""Authenticated HTTP boundary for the existing durable RT1 media coordinator.

The real worker must inject an actual SenderBackend. This adapter never
manufactures a remote media STOP receipt or starts an RTP sender itself.
"""
from __future__ import annotations

import hmac
import json
from http.server import BaseHTTPRequestHandler
from urllib.parse import urlsplit

from rt1_media_worker_lifecycle import MediaLifecycle, StopReceipt


def handler_for(lifecycle: MediaLifecycle, token: str):
    if not token:
        raise ValueError("RT1 worker authentication token is required")

    class Handler(BaseHTTPRequestHandler):
        def _send(self, status: int, body: dict):
            raw = json.dumps(body, separators=(",", ":")).encode("utf-8")
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)

        def _authorized(self) -> bool:
            supplied = self.headers.get("Authorization", "")
            return hmac.compare_digest(supplied.encode(), ("Bearer " + token).encode())

        def _request(self, method: str):
            if not self._authorized():
                self._send(401, {"error": "unauthorized"})
                return
            parts = urlsplit(self.path)
            if parts.query or parts.fragment:
                self._send(400, {"error": "invalid request"})
                return
            segments = parts.path.strip("/").split("/")
            if len(segments) != 4 or segments[:3] != ["v1", "avatar", "operations"]:
                self._send(404, {"error": "not found"})
                return
            operation_id = segments[3]
            if not operation_id or len(operation_id) > 128 or not all(
                char.isascii() and (char.isalnum() or char in "-_") for char in operation_id
            ):
                self._send(400, {"error": "invalid operation id"})
                return
            try:
                length = int(self.headers.get("Content-Length", "0"))
                if length < 1 or length > 1024:
                    raise ValueError("invalid request size")
                payload = json.loads(self.rfile.read(length))
                generation = payload["generation"]
                if type(generation) is not int or generation < 1:
                    raise ValueError("invalid generation")
            except (ValueError, KeyError, TypeError, json.JSONDecodeError):
                self._send(400, {"error": "invalid request"})
                return
            try:
                if method == "PUT":
                    session_id = lifecycle.create(operation_id, generation)
                    self._send(200, {"operation_id": operation_id, "session_id": session_id})
                else:
                    receipt = lifecycle.cancel(operation_id, generation)
                    if receipt is None:
                        # Unknown allocation / pending CREATE is NOT confirmed STOP.
                        self._send(202, {"operation_id": operation_id, "stop_confirmed": False})
                    else:
                        self._send(200, {
                            "operation_id": receipt.operation_id,
                            "session_id": receipt.session_id,
                            "generation": receipt.generation,
                            "audio_stopped": receipt.audio_stopped,
                            "video_stopped": receipt.video_stopped,
                        })
            except (RuntimeError, KeyError):
                self._send(409, {"error": "unresolved media allocation", "stop_confirmed": False})
            except Exception:
                self._send(503, {"error": "worker failure", "stop_confirmed": False})

        def do_PUT(self):
            self._request("PUT")

        def do_DELETE(self):
            self._request("DELETE")

    return Handler
