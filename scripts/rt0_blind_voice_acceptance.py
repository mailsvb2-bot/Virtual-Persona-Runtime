#!/usr/bin/env python3
"""Offline blind Russian-voice comparison. Never a production-release verdict.

Packages only fixed-text, WAV16 candidate previews. The examiner sees neutral
A/B/C labels, not engines, and must explicitly submit human listening scores.
Rights are independently reviewed; generated audio alone grants no permission.
"""
from __future__ import annotations

import argparse
from array import array
import hashlib
import html
import json
import math
import secrets
import shutil
import sys
import wave
from pathlib import Path

from rt0_multilingual_voice_audition import AUDITION_TEXT, ENGINES, MAX_PCM_BYTES

SCHEMA = "rt0-blind-russian-voice-audition-1"
SCORES = ("russian_nativeness", "pronunciation", "prosody", "timbre", "warmth")
TEXT_SHA256 = hashlib.sha256(AUDITION_TEXT.encode("utf-8")).hexdigest()
MAX_PREVIEWS = 6


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_json(path: Path) -> dict:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError("expected JSON object")
    return data


def valid_preview(wav_path: Path, metadata: dict, name: str) -> dict:
    if not wav_path.is_file() or wav_path.stat().st_size > MAX_PCM_BYTES + 44:
        raise ValueError("WAV is missing or exceeds bounded preview size")
    if (metadata.get("engine") != name or metadata.get("file") != wav_path.name
            or metadata.get("audition_text_sha256") != TEXT_SHA256
            or metadata.get("canonical_playback_proven") is not False):
        raise ValueError("preview has no matching fixed-Russian-text provenance")
    with wave.open(str(wav_path), "rb") as wav:
        if (wav.getcomptype() != "NONE" or wav.getsampwidth() != 2
                or wav.getnchannels() != 1
                or wav.getframerate() not in (22050, 24000, 32000, 44100, 48000)
                or not 0.25 < wav.getnframes() / wav.getframerate() <= 35):
            raise ValueError("unsupported or excessive audition audio")
        length = wav.getnframes() / wav.getframerate()
        pcm = array("h")
        pcm.frombytes(wav.readframes(wav.getnframes()))
        if sys.byteorder != "little":
            pcm.byteswap()
        if not pcm:
            raise ValueError("empty preview audio")
        # Technical sanity is not speech intelligibility or human approval.
        mean = sum(pcm) / len(pcm)
        mean_square = sum(float(x) * x for x in pcm) / len(pcm)
        ac_rms = math.sqrt(max(0.0, mean_square - mean * mean)) / 32768.0
        clipped_ratio = sum(abs(x) >= 32700 for x in pcm) / len(pcm)
        if ac_rms < 0.003 or clipped_ratio > 0.10:
            raise ValueError("silent, DC-only or severely clipped preview")
    if not isinstance(metadata.get("seconds"), (int, float)) or (
            abs(length - metadata["seconds"]) > 0.03):
        raise ValueError("preview duration does not match its generation evidence")
    return {"sha256": sha256(wav_path), "duration_seconds": round(length, 3),
            "ac_rms_dbfs": round(20 * math.log10(ac_rms), 2),
            "clipped_sample_ratio": round(clipped_ratio, 5)}


def render_browser(labels: list[str], session: str) -> str:
    pieces = []
    for label in labels:
        fields = "".join(
            f'<label>{html.escape(d.replace("_", " "))} (1–5)'
            f'<select required name="{label}_{d}"><option value="">Выберите оценку</option>'
            + "".join(f'<option value="{n}">{n}</option>' for n in range(1, 6))
            + '</select></label>'
            for d in SCORES
        )
        pieces.append(
            f'<section><h2>Вариант {label}</h2><audio controls preload="none" '
            f'src="{label}.wav"></audio><div class="grid">{fields}</div>'
            f'<label>Комментарии <textarea name="{label}_notes" rows="2"></textarea></label></section>'
        )
    js = """
const labels = LABELS;
const metrics = METRICS;
document.getElementById("review").addEventListener("submit", event => {
  event.preventDefault();
  const form = event.currentTarget;
  if (!form.reportValidity()) return;
  const ratings = labels.map(label => ({
    blind_label: label,
    scores: Object.fromEntries(metrics.map(metric =>
      [metric, Number(form.elements.namedItem(label + "_" + metric).value)])),
    notes: form.elements.namedItem(label + "_notes").value.slice(0, 500)
  }));
  const payload = {
    schema_version: "rt0-blind-listener-score-1",
    session_id: SESSION,
    human_listening_attested: true,
    reviewer_key: form.elements.namedItem("reviewer_key").value.trim(),
    ratings
  };
  const blob = new Blob([JSON.stringify(payload, null, 2)], {type: "application/json"});
  const link = document.createElement("a");
  link.href = URL.createObjectURL(blob);
  link.download = "blind-listening-" + SESSION + ".json";
  link.click();
  setTimeout(() => URL.revokeObjectURL(link.href), 1000);
});
""".replace("LABELS", json.dumps(labels)).replace("METRICS", json.dumps(SCORES)).replace(
        "SESSION", json.dumps(session))
    return ("<!doctype html><html lang=\"ru\"><meta charset=\"utf-8\">"
            "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">"
            "<title>Слепое сравнение русских голосов</title>"
            "<style>body{font:16px system-ui;margin:2rem auto;max-width:870px;padding:0 1rem}"
            "section{padding:1rem 0;border-bottom:1px solid #aaa}"
            "audio{display:block;width:min(100%,600px);margin:1rem 0}"
            ".grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(230px,1fr));gap:1rem}"
            "label{display:grid;gap:.3rem;margin:.4rem 0}"
            "select,textarea,button{padding:.65rem;font:inherit}</style>"
            "<h1>Сравнение русских голосов вслепую</h1>"
            "<p>Все варианты произносят ОДИН и тот же текст. "
            "Оценивайте только реально услышанную речь; буквам не соответствуют оценки качества. "
            "Это не тест синхронизации лица и не коммерческая лицензия.</p>"
            f"<blockquote>{html.escape(AUDITION_TEXT)}</blockquote>"
            "<form id=\"review\">" + "".join(pieces)
            + '<label>Ваш постоянный псевдоним для этого прослушивания (не имя) '
            '<input name="reviewer_key" minlength="3" maxlength="64" required '
            'autocomplete="off"></label>'
            '<label><input required type="checkbox"> Я лично прослушал(а) каждый голос '
            'в наушниках и подтверждаю свои оценки.</label>'
            '<button type="submit">Сохранить протокол оценок (JSON)</button></form>'
            f"<script>{js}</script></html>")


def prepare(source: Path, destination: Path, manifest_file: Path) -> dict:
    candidates = []
    for name in ENGINES:
        audio, record = source / f"{name}.wav", source / f"{name}.json"
        if audio.is_file() or record.is_file():
            if not audio.is_file() or not record.is_file():
                raise ValueError(f"incomplete candidate for {name}")
            metadata = read_json(record)
            identity = valid_preview(audio, metadata, name)
            candidates.append((name, audio, sha256(record), identity))
    if not 2 <= len(candidates) <= MAX_PREVIEWS:
        raise ValueError("blind comparison needs at least two distinct voice candidates")
    # Keep engine identity in a SEPARATE manifest, not the files the listener sees.
    if manifest_file.resolve().parent == destination.resolve():
        raise ValueError("private mapping must remain outside listener package")
    if destination.resolve() == source.resolve():
        raise ValueError("never overwrite the original voice evidence")
    if destination.exists() and any(destination.iterdir()):
        raise ValueError("listener package must start empty")
    if manifest_file.exists():
        raise ValueError("private manifest already exists; do not overwrite a past session")
    secrets.SystemRandom().shuffle(candidates)
    session = secrets.token_hex(16)
    manifest = {
        "schema_version": SCHEMA,
        "session_id": session,
        "text_sha256": TEXT_SHA256,
        "candidates": [],
        "release_ready": False,
    }
    destination.mkdir(parents=True, exist_ok=True)
    labels = []
    for i, (name, audio, sidecar_sha, ident) in enumerate(candidates):
        label = chr(ord("A") + i)
        labels.append(label)
        shutil.copyfile(audio, destination / f"{label}.wav")
        manifest["candidates"].append({
            "blind_label": label, "engine": name,
            "audio_sha256": ident["sha256"], "sidecar_sha256": sidecar_sha,
            "duration_seconds": ident["duration_seconds"],
            "ac_rms_dbfs": ident["ac_rms_dbfs"],
            "clipped_sample_ratio": ident["clipped_sample_ratio"],
        })
    (destination / "index.html").write_text(render_browser(labels, session), encoding="utf-8")
    manifest_file.parent.mkdir(parents=True, exist_ok=True)
    manifest_file.write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8")
    return {"session_id": session, "candidates": len(labels),
            "listener_file": str(destination / "index.html"),
            "private_mapping": str(manifest_file), "release_ready": False}


def assess(manifest_file: Path, package: Path, listener_files: list[Path],
           rights_file: Path) -> dict:
    manifest = read_json(manifest_file)
    if (manifest.get("schema_version") != SCHEMA
            or manifest.get("text_sha256") != TEXT_SHA256):
        raise ValueError("unsupported or modified fixed-text manifest")
    voices = manifest.get("candidates")
    if not isinstance(voices, list) or not 2 <= len(voices) <= MAX_PREVIEWS:
        raise ValueError("invalid voice identities")
    labels = {c["blind_label"] for c in voices}
    if len(labels) != len(voices):
        raise ValueError("duplicate blind labels")
    for voice in voices:
        if sha256(package / f"{voice['blind_label']}.wav") != voice["audio_sha256"]:
            raise ValueError("listener audio no longer matches the sealed manifest")
    if len(listener_files) < 2:
        raise ValueError("a single listener is not sufficient for voice selection")
    records = []
    listener_keys = set()
    listener_digests = set()
    for file in listener_files:
        digest = sha256(file)
        if digest in listener_digests:
            raise ValueError("duplicate listening file is not a second human")
        listener_digests.add(digest)
        item = read_json(file)
        listener_key = item.get("reviewer_key")
        if not isinstance(listener_key, str) or not 3 <= len(listener_key.strip()) <= 64:
            raise ValueError("reviewer pseudonym required for independent listening")
        if listener_key.strip() in listener_keys:
            raise ValueError("duplicate listener identity must not count twice")
        listener_keys.add(listener_key.strip())
        if (item.get("schema_version") != "rt0-blind-listener-score-1"
                or item.get("session_id") != manifest["session_id"]
                or item.get("human_listening_attested") is not True):
            raise ValueError("unattested or cross-session listening data")
        scores = item.get("ratings")
        if not isinstance(scores, list) or {x.get("blind_label") for x in scores} != labels:
            raise ValueError("missing or duplicated blind voice ratings")
        if len(scores) != len(labels):
            raise ValueError("invalid repeated voice rating")
        for row in scores:
            values = row.get("scores")
            if not isinstance(values, dict) or set(values) != set(SCORES):
                raise ValueError("incomplete quality scores")
            if any(type(n) is not int or not 1 <= n <= 5 for n in values.values()):
                raise ValueError("human scores must be integral 1–5")
        records.append({x["blind_label"]: x["scores"] for x in scores})
    rights = read_json(rights_file)
    if rights.get("schema_version") != "rt0-voice-rights-review-1":
        raise ValueError("rights review missing or unsupported")
    legal = rights.get("candidates")
    if not isinstance(legal, dict) or set(legal) != {x["engine"] for x in voices}:
        raise ValueError("rights review must name every evaluated engine")
    results = []
    for voice in voices:
        engine, label = voice["engine"], voice["blind_label"]
        evidence = legal[engine]
        if not isinstance(evidence, dict):
            raise ValueError("invalid rights declaration")
        approved = (evidence.get("commercial_use_confirmed") is True
                    and evidence.get("therapeutic_context_confirmed") is True
                    and evidence.get("voice_reference_consent_confirmed") is True
                    and isinstance(evidence.get("reviewer"), str)
                    and bool(evidence["reviewer"].strip())
                    and isinstance(evidence.get("evidence_url"), str)
                    and evidence["evidence_url"].startswith("https://"))
        averages = {
            metric: round(sum(row[label][metric] for row in records) / len(records), 2)
            for metric in SCORES
        }
        score = round(sum(averages.values()) / len(averages), 2)
        qualifies = approved and all(averages[k] >= 4 for k in SCORES)
        results.append({
            "engine": engine, "blind_label": label, "audio_sha256": voice["audio_sha256"],
            "human_review_count_unverified": len(records), "mean_scores": averages,
            "average": score, "rights_review_declared": bool(approved),
            "eligible_for_live_trial": bool(qualifies),
        })
    qualified = sorted((x for x in results if x["eligible_for_live_trial"]),
                       key=lambda x: (-x["average"], x["engine"]))
    return {
        "schema_version": "rt0-blind-voice-decision-1",
        "session_id": manifest["session_id"],
        "text_sha256": TEXT_SHA256,
        "selected_for_live_trial": qualified[0]["engine"] if qualified else None,
        "results": sorted(results, key=lambda x: x["engine"]),
        "release_ready": False,
        "human_identity_independently_verified": False,
        "canonical_audio_playout_proven": False,
        "avatar_voice_appearance_fit_proven": False,
        "disposition": "HUMAN_VOICE_SCREEN_ONLY_NOT_RT0_RELEASE",
    }


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    subs = p.add_subparsers(dest="command", required=True)
    make = subs.add_parser("prepare")
    make.add_argument("--source", type=Path, required=True)
    make.add_argument("--package", type=Path, required=True)
    make.add_argument("--private-manifest", type=Path, required=True)
    review = subs.add_parser("assess")
    review.add_argument("--package", type=Path, required=True)
    review.add_argument("--private-manifest", type=Path, required=True)
    review.add_argument("--listener", action="append", type=Path, required=True)
    review.add_argument("--rights", type=Path, required=True)
    review.add_argument("--report", type=Path, required=True)
    args = p.parse_args()
    try:
        if args.command == "prepare":
            answer = prepare(args.source, args.package, args.private_manifest)
        else:
            answer = assess(args.private_manifest, args.package, args.listener, args.rights)
            if args.report.exists():
                raise ValueError("do not overwrite a previous listening decision")
            args.report.parent.mkdir(parents=True, exist_ok=True)
            args.report.write_text(json.dumps(answer, ensure_ascii=False, indent=2),
                                   encoding="utf-8")
    except (ValueError, OSError, KeyError, TypeError, wave.Error) as exc:
        p.error(str(exc))
    print(json.dumps(answer, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
