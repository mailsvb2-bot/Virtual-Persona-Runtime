"""Isolated three-way native-Russian voice audition, never a production TTS port.

The existing Silero synthetic (MIT) reference is *only* a test fixture: its
pronunciation was rejected by the user, and cloning it cannot establish that a
model has native Russian diction. Qwen VoiceDesign is separately tested with
an explicit native-Russian female voice description and no reference audio.

No provider keys, browser data, user speech or D-ID audio can enter this run.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import time
import wave

AUDITION_TEXT = (
    "Здравствуйте! Рада вас слышать. "
    "Давайте спокойно разберёмся, что произошло, и найдём решение."
)
TEST_REFERENCE_SHA256 = "63546f12cab0b91685927ed92f621b70bb76bb8e600c8eb3fbdf11634de7be70"
TEST_REFERENCE_TEXT = (
    "Здравствуйте! Меня зовут Екатерина. Я рада вас слышать. "
    "Давайте поговорим спокойно и разберёмся, как я могу вам помочь."
)
QWEN_BASE = "Qwen/Qwen3-TTS-12Hz-0.6B-Base"
QWEN_DESIGN = "Qwen/Qwen3-TTS-12Hz-1.7B-VoiceDesign"
DESIGN_INSTRUCTION = (
    "A woman who is a native speaker of Russian from Moscow. "
    "Beautiful clear literary Russian with no foreign accent, "
    "natural unstressed vowel reduction, warm and composed adult voice, "
    "gentle compassionate conversational intonation. "
    "Normal speaking speed, not theatrical, not a cartoon."
)
MAX_AUDIO_SECONDS = 35
MAX_PCM_BYTES = 4 * 1024 * 1024
ENGINES = ("chatterbox-v3", "qwen-base", "qwen-design")


def validate_reference(path: Path) -> None:
    import hashlib
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    if digest != TEST_REFERENCE_SHA256:
        raise ValueError("audition reference must be exact permitted synthetic voice fixture")
    with wave.open(str(path), "rb") as audio:
        if (
            audio.getnchannels() != 1
            or audio.getframerate() != 24000
            or audio.getsampwidth() != 2
            or not 1 < audio.getnframes() / audio.getframerate() < 15
        ):
            raise ValueError("reference format, size or duration invalid")


def checked_wav(samples, sample_rate: int) -> bytes:
    import numpy as np

    data = np.asarray(samples, dtype=np.float32).reshape(-1)
    if sample_rate not in (22050, 24000, 32000, 44100, 48000):
        raise ValueError("unsupported sample rate")
    if not np.isfinite(data).all() or not .25 < len(data) / sample_rate <= MAX_AUDIO_SECONDS:
        raise ValueError("invalid audio duration or nonfinite output")
    audio = (np.clip(data, -1, 1) * 32767).astype("<i2", copy=False).tobytes()
    if len(audio) > MAX_PCM_BYTES:
        raise ValueError("audio output exceeds bounded PCM size")
    output = io.BytesIO()
    with wave.open(output, "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(sample_rate)
        wav.writeframes(audio)
    return output.getvalue()


def generate(backend: str, reference: Path | None):
    # Reject unsupported providers and arbitrary audio before importing optional
    # inference dependencies, downloading weights or touching any runtime state.
    if backend not in ENGINES:
        raise ValueError("unsupported audition engine")
    if backend in ("chatterbox-v3", "qwen-base") and reference is None:
        raise ValueError("synthetic Russian test reference required")
    if backend == "qwen-design" and reference is not None:
        raise ValueError("VoiceDesign is reference-free")
    if reference is not None:
        validate_reference(reference)

    import torch

    torch.set_num_threads(min(4, os.cpu_count() or 1))
    torch.manual_seed(2137)
    if backend == "chatterbox-v3":
        if reference is None:
            raise ValueError("Chatterbox v3 requires permitted Russian female reference")
        validate_reference(reference)
        from chatterbox.mtl_tts import ChatterboxMultilingualTTS

        engine = ChatterboxMultilingualTTS.from_pretrained(device="cpu", t3_model="v3")
        with torch.inference_mode():
            wav = engine.generate(
                AUDITION_TEXT,
                language_id="ru",
                audio_prompt_path=str(reference),
                cfg_weight=0.25,
                exaggeration=0.45,
            )
        return wav.detach().cpu().float().numpy(), engine.sr
    if backend in ("qwen-base", "qwen-design"):
        from qwen_tts import Qwen3TTSModel
        import numpy as np

        engine = Qwen3TTSModel.from_pretrained(
            QWEN_BASE if backend == "qwen-base" else QWEN_DESIGN,
            device_map="cpu",
            dtype=torch.float32,
            attn_implementation="eager",
        )
        with torch.inference_mode():
            if backend == "qwen-base":
                if reference is None:
                    raise ValueError("Qwen Base requires permitted synthetic Russian reference")
                validate_reference(reference)
                wavs, sr = engine.generate_voice_clone(
                    text=AUDITION_TEXT,
                    language="Russian",
                    ref_audio=str(reference),
                    ref_text=TEST_REFERENCE_TEXT,
                    x_vector_only_mode=False,
                    max_new_tokens=288,
                )
            else:
                if reference is not None:
                    raise ValueError("Qwen VoiceDesign must be zero-shot with no reference")
                wavs, sr = engine.generate_voice_design(
                    text=AUDITION_TEXT,
                    language="Russian",
                    instruct=DESIGN_INSTRUCTION,
                    max_new_tokens=288,
                )
        return np.asarray(wavs[0], dtype=np.float32), sr
    raise ValueError("unsupported audition engine")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", required=True, choices=ENGINES)
    parser.add_argument("--reference", type=Path)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    if args.engine in ("chatterbox-v3", "qwen-base") and args.reference is None:
        parser.error("the selected model requires an allowed Russian test voice sample")
    started = time.monotonic()
    data, rate = generate(args.engine, args.reference)
    wav = checked_wav(data, int(rate))
    args.output_dir.mkdir(parents=True, exist_ok=True)
    path = args.output_dir / (args.engine + ".wav")
    path.write_bytes(wav)
    # Some inference APIs return (1, samples) rather than (samples,). WAV is
    # the canonical evidence: len(data) would report 0.0 s for Chatterbox.
    with wave.open(io.BytesIO(wav), "rb") as confirmed:
        duration_seconds = confirmed.getnframes() / confirmed.getframerate()
    result = {
        "engine": args.engine,
        "audition_text_sha256": hashlib.sha256(AUDITION_TEXT.encode("utf-8")).hexdigest(),
        "seconds": round(duration_seconds, 2),
        "wall_seconds": round(time.monotonic() - started, 2),
        "rate_hz": int(rate),
        "reference_type": "synthetic_mit_sample" if args.reference else "none",
        "file": path.name,
        "canonical_playback_proven": False,
    }
    (args.output_dir / (args.engine + ".json")).write_text(
        json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    print("AUDITION_RESULT=" + json.dumps(result, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    main()
