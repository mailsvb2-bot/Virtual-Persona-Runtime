"""Opt-in lightweight Russian Piper synthesis via the existing secured Echo bridge.

Requires pip install piper-tts and a locally provisioned .onnx/.onnx.json pair.
Never downloads weights at request time. The provider identifier is currently
the existing OpenAI-compatible local TTS wire contract.
"""
import os
import io
import time
import wave
from pathlib import Path

from metrotrance_tts_bridge import make_handler, validate_echo_wav
from http.server import ThreadingHTTPServer


def stage_short_espeak_data(source_dir):
    """Work around Windows eSpeak's fixed-size native path buffer.

    The bundled data is valid, but a long/non-ASCII install path may cause
    eSpeak to fall back to its compiled-in CI builder path. Copy data into a
    unique short temporary directory owned by this process, never the repo.
    """
    import shutil
    import tempfile

    source = Path(source_dir).resolve(strict=True)
    if not (source / "phontab").is_file():
        raise ValueError("Piper bundled espeak-ng-data/phontab is missing")
    workspace = tempfile.TemporaryDirectory(prefix="vpr-es-")
    short_dir = Path(workspace.name) / "espeak-ng-data"
    try:
        if len(os.fsencode(str(short_dir))) > 100:
            raise ValueError("Windows TEMP path is too long for eSpeak; use a shorter TEMP directory")
        shutil.copytree(source, short_dir)
        if not (short_dir / "phontab").is_file():
            raise ValueError("Piper eSpeak staging did not preserve phontab")
        return workspace, short_dir
    except BaseException:
        workspace.cleanup()
        raise


def make_piper_synthesizer(model_path):
    import io
    import threading
    import wave

    model = Path(model_path).resolve(strict=True)
    if model.suffix != ".onnx" or not Path(str(model) + ".json").is_file():
        raise ValueError("Piper requires local .onnx and .onnx.json files")
    from piper import PiperVoice
    espeak_workspace = None
    if os.name == "nt":
        from piper.phonemize_espeak import ESPEAK_DATA_DIR
        espeak_workspace, espeak_dir = stage_short_espeak_data(ESPEAK_DATA_DIR)
        try:
            voice = PiperVoice.load(str(model), espeak_data_dir=espeak_dir)
        except BaseException:
            espeak_workspace.cleanup()
            raise
    else:
        voice = PiperVoice.load(str(model))
    lock = threading.Lock()

    def synthesize(text):
        # Retain the private staged eSpeak files until this synth closure dies.
        _ = espeak_workspace
        with lock:
            output = io.BytesIO()
            with wave.open(output, "wb") as wav:
                voice.synthesize_wav(text, wav)
            data = output.getvalue()
            if not validate_echo_wav(data):
                raise RuntimeError("Piper produced Echo-incompatible WAV")
            return data

    return synthesize



def warmup_piper(synthesize):
    """Warm the actual synthesis path before accepting voice traffic.

    RTF is processing seconds divided by generated audio seconds.
    No user phrase or audio is persisted.
    """
    start = time.perf_counter()
    audio = synthesize("Здравствуйте. Я готова говорить.")
    elapsed = time.perf_counter() - start
    with wave.open(io.BytesIO(audio), "rb") as source:
        duration = source.getnframes() / source.getframerate()
    if duration <= 0:
        raise RuntimeError("Piper warmup produced no audio")
    return round(elapsed / duration, 4)


PINNED_PIPER_VOICES = ("ru_RU-irina-medium", "ru_RU-denis-medium")


def check_selected_voice(model, selected):
    """Fail closed when the declared speech voice differs from the verified model."""
    if selected is None:
        return
    stem = os.path.basename(model)
    if stem.endswith(".onnx"):
        stem = stem[:-5]
    if selected not in PINNED_PIPER_VOICES or stem != selected:
        raise SystemExit("VPR_PIPER_VOICE does not match the pinned Piper model")


def main():
    model = os.environ.get("VPR_PIPER_MODEL")
    token = os.environ.get("VPR_METROTRANCE_TTS_TOKEN", "")
    port = int(os.environ.get("VPR_METROTRANCE_TTS_PORT", "8897"))
    if not model:
        raise SystemExit("VPR_PIPER_MODEL is required")
    check_selected_voice(model, os.environ.get("VPR_PIPER_VOICE"))
    synth = make_piper_synthesizer(model)
    rtf = warmup_piper(synth)
    server = ThreadingHTTPServer(
        ("127.0.0.1", port),
        make_handler(token, synth, provider_id="piper", health_status={"ready": True, "rtf": rtf}),
    )
    try:
        print("Piper TTS loopback ready", flush=True)
        server.serve_forever()
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
