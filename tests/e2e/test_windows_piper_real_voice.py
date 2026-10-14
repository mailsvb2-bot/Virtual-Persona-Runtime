"""Real CPU Piper inference on Windows; does not claim human audible playback.

Uses the pinned provisioned model; tests actual synthesis and authenticated
HTTP speech handoff. No D-ID, owner identity, microphone, or real speaker used.
"""
from __future__ import annotations
import argparse
import http.client
import io
import json
import sys
import threading
import wave
from http.server import ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from metrotrance_tts_bridge import make_handler, validate_echo_wav
from piper_tts_bridge import make_piper_synthesizer


def request(port: int, token: str, payload: dict) -> tuple[int, bytes]:
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=45)
    try:
        connection.request(
            "POST", "/v1/audio/speech",
            json.dumps(payload, ensure_ascii=False).encode("utf-8"),
            {"Authorization": "Bearer " + token, "Content-Type": "application/json"},
        )
        response = connection.getresponse()
        return response.status, response.read()
    finally:
        connection.close()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", type=Path, required=True)
    options = parser.parse_args()
    synthesize = make_piper_synthesizer(str(options.model))
    token = "windows-real-voice-validation-random-test-only"
    server = ThreadingHTTPServer(
        ("127.0.0.1", 0),
        make_handler(token, synthesize, provider_id="piper"),
    )
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    payload = {
        "model": "piper", "voice": "local", "response_format": "wav",
        "input": "Привет! Это настоящий тест русского голоса.",
    }
    try:
        wrong, _ = request(server.server_port, token + "-incorrect", payload)
        assert wrong == 401, f"wrong bearer token accepted: {wrong}"
        mismatch, _ = request(server.server_port, token, {**payload, "model": "other"})
        assert mismatch == 400, f"wrong voice/model accepted: {mismatch}"
        status, audio = request(server.server_port, token, payload)
        assert status == 200, f"real Piper synthesis failed: {status}"
        assert validate_echo_wav(audio), "real synthesized WAV failed Echo format validation"
        with wave.open(io.BytesIO(audio), "rb") as wav:
            samples = wav.readframes(wav.getnframes())
            seconds = wav.getnframes() / wav.getframerate()
            assert wav.getsampwidth() == 2 and wav.getnchannels() == 1
            assert wav.getframerate() == 22050
            assert seconds > 0.25
            assert any(samples[i:i+2] != b"\x00\x00" for i in range(0, len(samples), 2))
            print(f"Real Windows Piper: WAV validated; duration={seconds:.2f}s, {wav.getframerate()}Hz mono PCM16")
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=3)


if __name__ == "__main__":
    main()
