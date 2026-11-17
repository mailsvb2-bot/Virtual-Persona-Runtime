"""GPU-free fail-closed MuseTalk v1.5 deployment preflight.

Checks the installed upstream API and approved local assets before spending
GPU time. Does not import torch, decode models or unpickle Persona data.
"""
from __future__ import annotations

import argparse
import ast
import json
from pathlib import Path
import re
import subprocess
import sys

from rt1_musetalk_renderer import prepared_avatar_digest

APIS = {
    "musetalk/utils/utils.py": {
        "load_all_model": ("unet_model_path", "unet_config", "device"),
        "datagen": ("whisper_chunks", "vae_encode_latents", "batch_size", "device"),
    },
    "musetalk/utils/audio_processor.py": {
        "AudioProcessor.get_whisper_chunk": (
            "whisper_input_features", "device", "weight_dtype", "whisper",
            "librosa_length", "fps", "audio_padding_length_left",
            "audio_padding_length_right",
        ),
    },
    "musetalk/utils/blending.py": {
        "get_image_blending": (
            "ori_frame", "res_frame", "bbox", "mask", "mask_crop_box"
        ),
    },
}
MODEL_FILES = (
    "models/musetalkV15/unet.pth",
    "models/musetalkV15/musetalk.json",
    "models/whisper/config.json",
    "models/whisper/preprocessor_config.json",
    "models/whisper/pytorch_model.bin",
    "models/sd-vae/config.json",
    "models/sd-vae/diffusion_pytorch_model.bin",
)


def check_apis(root: Path) -> None:
    """Inspect source AST: no upstream imports or CUDA initialization."""
    for path, names in APIS.items():
        nodes = ast.parse((root / path).read_text(encoding="utf-8")).body
        for dotted, required in names.items():
            current = nodes
            fn = None
            for part in dotted.split("."):
                fn = next(
                    (n for n in current
                     if isinstance(n, (ast.ClassDef, ast.FunctionDef))
                     and n.name == part), None
                )
                if fn is None:
                    raise ValueError(f"missing upstream API {path}:{dotted}")
                current = fn.body
            if not isinstance(fn, ast.FunctionDef):
                raise ValueError(f"upstream API not callable {path}:{dotted}")
            args = [a.arg for a in fn.args.posonlyargs + fn.args.args]
            if "." in dotted:
                if not args or args[0] != "self":
                    raise ValueError(f"incompatible upstream API {path}:{dotted}")
                args = args[1:]
            if any(name not in args for name in required):
                raise ValueError(f"incompatible upstream API {path}:{dotted}")


def inspect(root: Path, avatar: Path, *, revision: str, digest: str) -> dict:
    if re.fullmatch(r"[0-9a-f]{40}", revision) is None:
        raise ValueError("approved MuseTalk revision must be full SHA-1")
    if re.fullmatch(r"[0-9a-f]{64}", digest) is None:
        raise ValueError("approved Persona digest must be SHA-256")
    root, avatar = root.resolve(strict=True), avatar.resolve(strict=True)
    if not root.is_dir() or not avatar.is_dir():
        raise ValueError("MuseTalk root and prepared avatar must be directories")
    def git(*args):
        return subprocess.check_output(
            ["git", "-C", str(root), *args], text=True,
            stderr=subprocess.DEVNULL, timeout=10
        ).strip()
    if git("rev-parse", "HEAD") != revision:
        raise ValueError("MuseTalk source revision differs from approved commit")
    if git("status", "--porcelain", "--untracked-files=all"):
        raise ValueError("modified or untracked MuseTalk source files (review build tree)")
    check_apis(root)
    for path in MODEL_FILES:
        file = root / path
        if file.is_symlink() or not file.is_file() or file.stat().st_size == 0:
            raise ValueError(f"missing or invalid MuseTalk model: {path}")
    for path in ("latents.pt", "coords.pkl", "mask_coords.pkl"):
        file = avatar / path
        if file.is_symlink() or not file.is_file() or file.stat().st_size == 0:
            raise ValueError(f"missing or invalid prepared avatar: {path}")
    actual_digest = prepared_avatar_digest(avatar)
    if actual_digest != digest:
        raise ValueError("prepared avatar does not match independently approved digest")
    count = len(list((avatar / "full_imgs").glob("*.png")))
    if count != len(list((avatar / "mask").glob("*.png"))):
        raise ValueError("prepared full image and mask frame counts differ")
    return {
        "schema": "vpr.rt1.offline-gpu-preflight.v1",
        "upstream_revision": revision,
        "prepared_avatar_sha256": actual_digest,
        "prepared_frames": count,
        "upstream_apis_checked": True,
        "cuda_executed": False,
        "real_gpu_accepted": False,
        "browser_av_accepted": False,
        "production_release_approved": False,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--avatar-dir", type=Path, required=True)
    parser.add_argument("--expected-revision", required=True)
    parser.add_argument("--approved-avatar-sha256", required=True)
    a = parser.parse_args(argv)
    try:
        report = inspect(a.root, a.avatar_dir,
                         revision=a.expected_revision,
                         digest=a.approved_avatar_sha256)
    except (ValueError, OSError, SyntaxError, subprocess.SubprocessError) as exc:
        print(f"RT1 offline preflight FAILED: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(report, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
