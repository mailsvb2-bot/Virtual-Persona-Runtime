"""Generate a fixed, private-data-free three-voice Russian Silero audition.

This does NOT alter Owner Lab's chosen TTS or claim live D-ID playback. The
model is loaded only from an SHA256-pinned local file. PyTorch package loading
can execute bundled Python objects; never remove the digest guard.
"""
from __future__ import annotations

import argparse
import hashlib
import io
from pathlib import Path
import wave

MODEL_SHA256 = "0405777e332906f0644e08a680f7cfdc2137ea864090079c1fdd30a43c1b8761"
MODEL_NAME = "v5_cis_base_nostress.pt"
MODEL_URL = (
    "https://huggingface.co/adeshkin/silero-models-v5-cis-base-nostress/"
    "resolve/5f1a14e/" + MODEL_NAME
)
FEMALE_VOICES = ("ru_ekaterina", "ru_karina", "ru_vika")
SAMPLE_RATE = 24000
MIN_MODEL_BYTES = 80_000_000
MAX_MODEL_BYTES = 140 * 1024 * 1024
TEST_PHRASE = (
    "Здравствуйте! Меня зовут Екатерина. Я рада вас слышать. "
    "Давайте поговорим спокойно и разберёмся, как я могу вам помочь."
)


def verified_model(path: Path) -> bool:
    if not path.is_file() or not MIN_MODEL_BYTES <= path.stat().st_size <= MAX_MODEL_BYTES:
        return False
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest() == MODEL_SHA256


def pcm16_wav(audio, torch_module, *, sample_rate: int = SAMPLE_RATE) -> bytes:
    if not 8000 <= sample_rate <= 48000:
        raise ValueError("invalid voice sample rate")
    samples = audio.detach().cpu().flatten()
    if samples.numel() < sample_rate // 4 or samples.numel() > sample_rate * 60:
        raise ValueError("Silero audio duration outside canonical limits")
    if not bool(torch_module.isfinite(samples).all()):
        raise ValueError("Silero audio contains invalid samples")
    pcm = samples.to(dtype=torch_module.float32).clamp(-1.0, 1.0)
    pcm = pcm.mul(32767.0).to(dtype=torch_module.int16).numpy().tobytes()
    out = io.BytesIO()
    with wave.open(out, "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(sample_rate)
        wav.writeframes(pcm)
    return out.getvalue()


def load_verified_silero(model_file: Path):
    if not verified_model(model_file):
        raise ValueError("Silero CIS Base model hash or size mismatch")
    import torch

    torch.set_num_threads(min(4, max(1, __import__("os").cpu_count() or 1)))
    model = torch.package.PackageImporter(str(model_file)).load_pickle("tts_models", "model")
    model.to(torch.device("cpu"))
    return model, torch


def synthesize_samples(model, torch_module, voices=FEMALE_VOICES) -> dict[str, bytes]:
    results = {}
    for voice in voices:
        if voice not in FEMALE_VOICES:
            raise ValueError("unapproved or non-Russian voice")
        with torch_module.inference_mode():
            tensor = model.apply_tts(text=TEST_PHRASE, speaker=voice, sample_rate=SAMPLE_RATE)
        results[voice] = pcm16_wav(tensor, torch_module)
    return results


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    # All audio is a fixed, public test phrase; no conversation or user audio.
    model, torch_module = load_verified_silero(args.model)
    audio = synthesize_samples(model, torch_module)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    for name, wav in audio.items():
        target = args.output_dir / (name + ".wav")
        target.write_bytes(wav)
        print(f"SILERO_SAMPLE voice={name} bytes={len(wav)}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
