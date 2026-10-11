"""Opt-in lightweight Russian Piper synthesis via the existing secured Echo bridge.

Requires pip install piper-tts and a locally provisioned .onnx/.onnx.json pair.
Never downloads weights at request time. The provider identifier is currently
the existing OpenAI-compatible local TTS wire contract.
"""
import os
from pathlib import Path

from metrotrance_tts_bridge import make_handler, validate_echo_wav
from http.server import ThreadingHTTPServer


def make_piper_synthesizer(model_path):
    import io
    import threading
    import wave

    model = Path(model_path).resolve(strict=True)
    if model.suffix != ".onnx" or not Path(str(model) + ".json").is_file():
        raise ValueError("Piper requires local .onnx and .onnx.json files")
    from piper import PiperVoice
    voice = PiperVoice.load(str(model))
    lock = threading.Lock()

    def synthesize(text):
        with lock:
            output = io.BytesIO()
            with wave.open(output, "wb") as wav:
                voice.synthesize_wav(text, wav)
            data = output.getvalue()
            if not validate_echo_wav(data):
                raise RuntimeError("Piper produced Echo-incompatible WAV")
            return data

    return synthesize


def main():
    model = os.environ.get("VPR_PIPER_MODEL")
    token = os.environ.get("VPR_METROTRANCE_TTS_TOKEN", "")
    port = int(os.environ.get("VPR_METROTRANCE_TTS_PORT", "8897"))
    if not model:
        raise SystemExit("VPR_PIPER_MODEL is required")
    synth = make_piper_synthesizer(model)
    server = ThreadingHTTPServer(
        ("127.0.0.1", port),
        make_handler(token, synth, provider_id="piper"),
    )
    try:
        print("Piper TTS loopback ready", flush=True)
        server.serve_forever()
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
