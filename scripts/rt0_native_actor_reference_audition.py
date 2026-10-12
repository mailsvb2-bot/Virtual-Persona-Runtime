"""Audition Qwen Base with *recorded native Russian actresses*.

References: Langswap Dialogs (OpenRAIL), studios, actors' written permission.
This is evaluation ONLY: its license prohibits some downstream uses (including
medical advice). This script must never load user audio, provider credentials,
or be wired into production TtsPort/Echo without an additional rights review.

The dataset revision, exact public Russian utterances and speaker ids are
pinned, to prevent an uncontrolled voice becoming a cloned reference.
"""
from __future__ import annotations

import hashlib
import io
import json
import os
from pathlib import Path
import time
import wave

from rt0_multilingual_voice_audition import AUDITION_TEXT, checked_wav

DATASET_ID = "langswap/dialogs-ru-emotional-conversations"
DATASET_REVISION = "09b2b30d2afe5fe21d1dc191c4415ea76c16ef97"
QWEN_MODEL_ID = "Qwen/Qwen3-TTS-12Hz-0.6B-Base"
SAMPLE_RATE = 24000

# Exact ordinary spoken sentences in the original Dialogs parquet preview.
# Never fall back to a guessed/foreign speaker if a row is absent.
REFERENCE_SELECTION = {
    "masha": {
        "speaker_id": "M",
        "emotion": "neutral",
        "text": "Мне нравятся те, где кошки подражают известным персонажам или сценам из фильмов. Они такими умелыми оказываются!",
    },
    "sveta": {
        "speaker_id": "S",
        "emotion": "surprise",
        "text": "А какие еще инструменты могут помочь в управлении рисками?",
    },
}

MAX_REFERENCE_SECONDS = 13
MAX_REFERENCE_BYTES = 3 * 1024 * 1024


def select_audition_row(rows: list[dict], voice: str) -> dict:
    if voice not in REFERENCE_SELECTION:
        raise ValueError("reference voice is not authorized for this audition")
    expected = REFERENCE_SELECTION[voice]
    matches = [
        row for row in rows
        if all(str(row.get(key, "")) == value for key, value in expected.items())
    ]
    if len(matches) != 1:
        raise ValueError("pinned speaker, style or transcript missing or duplicated")
    return matches[0]


def decode_native_reference(row: dict):
    import numpy as np
    import soundfile as sf
    from scipy.signal import resample_poly
    from math import gcd

    audio = row.get("audio")
    if not isinstance(audio, dict) or not isinstance(audio.get("bytes"), bytes):
        raise ValueError("expected embedded, locally decoded dataset audio")
    blob = audio["bytes"]
    if not blob or len(blob) > MAX_REFERENCE_BYTES:
        raise ValueError("reference file is absent or exceeds bound")
    sound, source_rate = sf.read(io.BytesIO(blob), dtype="float32", always_2d=False)
    if sound.ndim == 2:
        if sound.shape[1] != 1:
            raise ValueError("multi-channel reference is not a verified single speaker")
        sound = sound[:, 0]
    if sound.ndim != 1 or not 2.5 <= len(sound) / source_rate <= MAX_REFERENCE_SECONDS:
        raise ValueError("wrong reference duration")
    if not np.isfinite(sound).all() or float(np.max(np.abs(sound))) > 1.0001:
        raise ValueError("reference contains invalid samples")
    if source_rate != SAMPLE_RATE:
        factor = gcd(source_rate, SAMPLE_RATE)
        sound = resample_poly(sound, SAMPLE_RATE // factor, source_rate // factor)
    sound = np.asarray(sound, dtype=np.float32)
    if not np.isfinite(sound).all() or not 2.5 <= len(sound) / SAMPLE_RATE <= MAX_REFERENCE_SECONDS:
        raise ValueError("resampled reference malformed")
    return sound, {
        "reference_sha256": hashlib.sha256(blob).hexdigest(),
        "reference_seconds": round(len(sound) / SAMPLE_RATE, 2),
        "reference_original_rate": int(source_rate),
        "ref_text": row["text"],
    }


def load_pinned_dialogs_preview(cache_dir: Path) -> list[dict]:
    import pyarrow.parquet as pq
    from huggingface_hub import hf_hub_download

    preview = hf_hub_download(
        repo_id=DATASET_ID,
        repo_type="dataset",
        revision=DATASET_REVISION,
        filename="preview.parquet",
        local_dir=str(cache_dir),
    )
    license_file = hf_hub_download(
        repo_id=DATASET_ID,
        repo_type="dataset",
        revision=DATASET_REVISION,
        filename="LICENSE.md",
        local_dir=str(cache_dir),
    )
    terms = Path(license_file).read_text(encoding="utf-8")
    # This grant is conditional, not equivalent to MIT/CC0; keep source terms.
    required = {
        "license": "OpenRAIL",
        "commercial grant": "including commercial purposes",
        "medical-use restriction": "To provide medical advice",
        "recorded actor consent": "written informed consent",
    }
    missing = [name for name, marker in required.items() if marker not in terms]
    if missing:
        # Report only which legal assurance is absent, never source audio.
        raise RuntimeError("source-rights gate missing: " + ", ".join(missing))
    return pq.read_table(
        preview, columns=["audio", "speaker_id", "emotion", "text"]
    ).to_pylist()


def run(output_dir: Path, cache_dir: Path):
    import torch
    import numpy as np
    from qwen_tts import Qwen3TTSModel

    rows = load_pinned_dialogs_preview(cache_dir)
    refs = {}
    for voice in REFERENCE_SELECTION:
        refs[voice] = decode_native_reference(select_audition_row(rows, voice))

    torch.set_num_threads(min(4, os.cpu_count() or 1))
    torch.manual_seed(2137)
    engine = Qwen3TTSModel.from_pretrained(
        QWEN_MODEL_ID,
        device_map="cpu",
        dtype=torch.float32,
        attn_implementation="eager",
    )
    output_dir.mkdir(parents=True, exist_ok=True)
    for voice, (source, meta) in refs.items():
        started = time.monotonic()
        with torch.inference_mode():
            results, rate = engine.generate_voice_clone(
                text=AUDITION_TEXT,
                language="Russian",
                ref_audio=(source, SAMPLE_RATE),
                ref_text=meta["ref_text"],
                x_vector_only_mode=False,
                max_new_tokens=288,
            )
        wav = checked_wav(np.asarray(results[0], dtype=np.float32), int(rate))
        filename = f"qwen-base-native-{voice}"
        with wave.open(io.BytesIO(wav), "rb") as confirmed:
            duration = confirmed.getnframes() / confirmed.getframerate()
            if duration < .25:
                raise RuntimeError("empty synthesized response")
        (output_dir / (filename + ".wav")).write_bytes(wav)
        evidence = {
            "voice": voice,
            "dataset_id": DATASET_ID,
            "dataset_revision": DATASET_REVISION,
            "dataset_license": "OpenRAIL; medical-advice use prohibited; audit-only",
            "rights_source": "LICENSE.md and informed actor consent",
            "speaker_id": REFERENCE_SELECTION[voice]["speaker_id"],
            "transcript_sha256": hashlib.sha256(meta["ref_text"].encode("utf-8")).hexdigest(),
            "reference_sha256": meta["reference_sha256"],
            "reference_seconds": meta["reference_seconds"],
            "reference_original_rate": meta["reference_original_rate"],
            "output_seconds": round(duration, 2),
            "inference_wall_seconds": round(time.monotonic() - started, 2),
            "synthetic_audio_played_to_avatar": False,
        }
        (output_dir / (filename + ".json")).write_text(
            json.dumps(evidence, indent=2, ensure_ascii=False), encoding="utf-8"
        )
        print("NATIVE_RU_AUDITION=" + json.dumps(evidence, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--cache-dir", type=Path, required=True)
    args = parser.parse_args()
    run(args.output_dir, args.cache_dir)
