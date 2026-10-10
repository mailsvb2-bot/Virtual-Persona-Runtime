"""Opt-in loopback OpenAI speech-compatible endpoint for an installed MetroTrance.

This adapter imports MetroTrance from a *separate local checkout*. It never edits
that repository or copies its voice samples into the VPR worktree.
Do not expose this listener to a network or mistake it for production Echo.
"""
from __future__ import annotations

import hmac
import io
import json
import os
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

MAX_REQUEST = 8192
MAX_TEXT = 4096
MAX_WAV_BYTES = 12 * 1024 * 1024


def make_synthesizer(checkout: Path, engine: str):
    checkout = checkout.resolve(strict=True)
    if not (checkout / "metrotrance" / "providers" / "qwen_tts.py").is_file():
        raise ValueError("METROTRANCE_ROOT must point to the MetroTrance source checkout")
    sys.path.insert(0, str(checkout))
    from metrotrance.config import get_settings
    settings = get_settings(checkout)
    if engine == "qwen":
        from metrotrance.providers.qwen_tts import QwenTTSProvider
        provider = QwenTTSProvider(settings)
    elif engine == "chatterbox":
        from metrotrance.providers.chatterbox_tts import ChatterboxTTSProvider
        provider = ChatterboxTTSProvider(settings)
    else:
        raise ValueError("engine must be qwen or chatterbox")
    reference = settings.voice_audio.resolve()
    transcript = settings.voice_transcript.resolve()
    if not reference.is_file():
        raise ValueError("MetroTrance reference.wav is missing")
    if engine == "qwen" and not transcript.is_file():
        raise ValueError("MetroTrance voice transcript is missing")
    reference_text = transcript.read_text(encoding="utf-8") if transcript.is_file() else ""
    lock = threading.Lock()

    def synthesize(text: str) -> bytes:
        # The studio workflow is disk-oriented. Keep its temporary outputs
        # isolated, choose one take for latency, and never persist caller text.
        with lock, tempfile.TemporaryDirectory(prefix="vpr-metrotrance-") as temp:
            files = provider.synthesize_chunks(
                [text], Path(temp), reference, reference_text,
                performance={"natural_takes": 1},
            )
            if len(files) != 1:
                raise RuntimeError("MetroTrance did not generate exactly one phrase")
            path = Path(files[0])
            if path.stat().st_size > MAX_WAV_BYTES:
                raise RuntimeError("generated audio exceeds size limit")
            data = path.read_bytes()
            if not data.startswith(b"RIFF") or data[8:12] != b"WAVE":
                raise RuntimeError("MetroTrance returned non-WAV audio")
            return data

    return synthesize


def make_handler(token: str, synthesizer):
    if not token or len(token) < 24:
        raise ValueError("VPR_METROTRANCE_TTS_TOKEN must contain at least 24 characters")

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, _format, *_args):
            # No phrases or credentials in HTTP logs.
            return

        def send_error_code(self, status: int, message: str):
            data = json.dumps({"error": message}).encode("utf-8")
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_POST(self):
            if self.path != "/v1/audio/speech":
                return self.send_error_code(404, "not_found")
            supplied = self.headers.get("Authorization", "")
            if not hmac.compare_digest(supplied, "Bearer " + token):
                return self.send_error_code(401, "unauthorized")
            size = self.headers.get("Content-Length", "")
            if not size.isdecimal() or not 0 < int(size) <= MAX_REQUEST:
                return self.send_error_code(413, "invalid_request_size")
            try:
                payload = json.loads(self.rfile.read(int(size)))
                phrase = payload.get("input")
                if not isinstance(phrase, str) or not phrase.strip() or len(phrase.encode("utf-8")) > MAX_TEXT:
                    return self.send_error_code(400, "invalid_input")
                if payload.get("response_format", "wav") != "wav":
                    return self.send_error_code(400, "wav_required")
                wav = synthesizer(phrase)
                if not isinstance(wav, bytes) or len(wav) > MAX_WAV_BYTES:
                    return self.send_error_code(502, "invalid_audio")
            except (ValueError, TypeError, UnicodeDecodeError):
                return self.send_error_code(400, "invalid_request")
            except Exception:
                return self.send_error_code(502, "synthesis_failed")
            self.send_response(200)
            self.send_header("Content-Type", "audio/wav")
            self.send_header("Content-Length", str(len(wav)))
            self.end_headers()
            self.wfile.write(wav)

    return Handler


def main():
    root = os.environ.get("METROTRANCE_ROOT")
    token = os.environ.get("VPR_METROTRANCE_TTS_TOKEN", "")
    engine = os.environ.get("VPR_METROTRANCE_ENGINE", "qwen")
    port = int(os.environ.get("VPR_METROTRANCE_TTS_PORT", "8897"))
    if not root:
        raise SystemExit("METROTRANCE_ROOT is required")
    synth = make_synthesizer(Path(root), engine)
    server = ThreadingHTTPServer(("127.0.0.1", port), make_handler(token, synth))
    print(f"MetroTrance local TTS listening at http://127.0.0.1:{port}/v1/audio/speech", flush=True)
    try:
        server.serve_forever()
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
