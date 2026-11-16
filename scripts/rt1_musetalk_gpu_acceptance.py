"""Opt-in REAL CUDA MuseTalk inference acceptance; never grants publication.

Run on the operator-approved GPU host with pinned Persona assets. Unlike
hermetic RTP fixtures this imports the real upstream MuseTalk/Whisper models,
runs repeated 400ms windows and measures synchronized GPU completion.
No voice, face pixels or credential values are written to evidence.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import sys
import time

from rt1_musetalk_renderer import (
    CHUNK_FRAMES, PCM_WINDOW_SAMPLES, VIDEO_FPS,
    renderer_from_environment, prepared_avatar_digest,
)


def evaluate_windows(latencies_ms: list[float], *, max_window_ms: float) -> dict:
    """A pass means model computation keeps up with 400ms PCM windows.

    Real browser A/V offset, WebRTC tracks and STOP still need separate
    independently verified release evidence.
    """
    if not latencies_ms or any(not 0 < ms < float("inf") for ms in latencies_ms):
        raise ValueError("positive measured inference samples required")
    if max_window_ms <= 0:
        raise ValueError("invalid inference deadline")
    mean_ms = statistics.mean(latencies_ms)
    p95_ms = sorted(latencies_ms)[max(0, (95 * len(latencies_ms) + 99) // 100 - 1)]
    return {
        "samples": len(latencies_ms),
        "mean_window_ms": round(mean_ms, 3),
        "p95_window_ms": round(p95_ms, 3),
        "worst_window_ms": round(max(latencies_ms), 3),
        "observed_processing_fps": round(1000 * CHUNK_FRAMES / mean_ms, 3),
        "meets_compute_deadline": max(latencies_ms) <= max_window_ms,
        "deadline_ms": max_window_ms,
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--windows", type=int, default=20)
    parser.add_argument("--max-window-ms", type=float, default=400.0)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    if not 5 <= args.windows <= 1000:
        parser.error("--windows must be between 5 and 1000")
    if args.output.exists():
        parser.error("output already exists: never overwrite earlier acceptance evidence")
    if os.environ.get("RT1_VIDEO_RENDERER") != "musetalk-v15":
        parser.error("requires explicit RT1_VIDEO_RENDERER=musetalk-v15")
    if args.output.is_symlink():
        parser.error("refuse symlink output")
    # Fail hard on missing real CUDA/model/approved assets. No mock hooks or
    # test-pattern fallbacks are used by this executable.
    start = time.perf_counter()
    renderer = renderer_from_environment()
    if renderer is None or not renderer.ready():
        raise RuntimeError("real CUDA renderer was not ready")
    model_load_ms = (time.perf_counter() - start) * 1000
    import numpy as np
    import torch

    times = []
    for frame_index in range(0, args.windows * CHUNK_FRAMES, CHUNK_FRAMES):
        # Non-silent deterministic PCM drives real Whisper->UNet->VAE; this
        # measures computational throughput, NOT quality of lip-sync.
        tick = np.arange(PCM_WINDOW_SAMPLES, dtype=np.int32)
        samples = ((tick * 43 + frame_index * 11) % 5000 - 2500).astype("<i2")
        torch.cuda.synchronize(renderer._device)
        t0 = time.perf_counter()
        frames = renderer._infer(samples, frame_index)
        torch.cuda.synchronize(renderer._device)
        ms = (time.perf_counter() - t0) * 1000
        if len(frames) != CHUNK_FRAMES or any(
            frame is None or frame.ndim != 3 or frame.shape[2] != 3
            for frame in frames
        ):
            raise RuntimeError("real model did not produce ten valid BGR frames")
        times.append(ms)
    result = evaluate_windows(times, max_window_ms=args.max_window_ms)
    result.update({
        "schema": "vpr.rt1.real-gpu-compute-acceptance.v1",
        "real_cuda_execution": True,
        "model": "musetalk-v15",
        "device_name": torch.cuda.get_device_name(renderer._device),
        "torch_cuda_version": torch.version.cuda,
        "persona_id_sha256": hashlib.sha256(
            renderer.persona_identity()[0].encode("utf-8")
        ).hexdigest(),
        "persona_version": renderer.persona_identity()[1],
        "prepared_assets_sha256": prepared_avatar_digest(
            os.environ["RT1_MUSETALK_AVATAR_DIR"]
        ),
        "startup_warmup_ms": round(model_load_ms, 3),
        "video_fps_target": VIDEO_FPS,
        "browser_av_offset_verified": False,
        "dual_track_stop_verified": False,
        "production_release_approved": False,
    })
    # Ensure evidence itself can never be taken as permission to publish.
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x", encoding="utf-8") as handle:
        json.dump(result, handle, sort_keys=True, indent=2, allow_nan=False)
        handle.write("\n")
    print(json.dumps({
        "evidence": str(args.output),
        "deadline_passed": result["meets_compute_deadline"],
        "browser_and_stop_still_required": True,
    }))
    return 0 if result["meets_compute_deadline"] else 2


if __name__ == "__main__":
    sys.exit(main())
