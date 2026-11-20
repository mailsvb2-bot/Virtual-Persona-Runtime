"""GPU-backed MuseTalk v1.5 Persona video, driven only by canonical 16k PCM.

The aiortc sender still owns CREATE/SDP/RTP/STOP; this module only renders
frames. No model downloads, fallback faces, policy, TTS or second media owner.
Requires an operator-prepared MuseTalk avatar and local CUDA model weights.
"""
from __future__ import annotations

import hashlib
import hmac
import asyncio
from fractions import Fraction
import logging
import os
from pathlib import Path
import queue
import re
import subprocess
import sys
import threading
import time

LOG = logging.getLogger("rt1.musetalk")
SAMPLE_RATE = 16000
VIDEO_FPS = 25
VIDEO_RTP_CLOCK_HZ = 90000
VIDEO_FRAME_PTS = VIDEO_RTP_CLOCK_HZ // VIDEO_FPS
VIDEO_TIME_BASE = Fraction(1, VIDEO_RTP_CLOCK_HZ)
SAMPLES_PER_FRAME = SAMPLE_RATE // VIDEO_FPS  # 40 ms / generated video frame
PCM_BYTES_PER_SAMPLE = 2  # PCM16LE mono
CHUNK_FRAMES = 10         # 400 ms of authorized PCM for Whisper context
PCM_WINDOW_SAMPLES = CHUNK_FRAMES * SAMPLES_PER_FRAME
PCM_WINDOW_BYTES = PCM_WINDOW_SAMPLES * PCM_BYTES_PER_SAMPLE
MAX_AUDIO_CHUNKS = 4
MAX_VIDEO_FRAMES = 25

# These are precisely the upstream model files loaded by the RT1 runtime.
# Match the independent offline preflight to prevent drift.
REQUIRED_MODEL_FILES = (
    "models/musetalkV15/unet.pth",
    "models/musetalkV15/musetalk.json",
    "models/whisper/config.json",
    "models/whisper/preprocessor_config.json",
    "models/whisper/pytorch_model.bin",
    "models/sd-vae/config.json",
    "models/sd-vae/diffusion_pytorch_model.bin",
)


def validate_frame_batch(frames, expected_shape=None):
    """Validate the sender's actual 8-bit BGR video contract, without pixels in evidence.

    This is a structural CUDA output check, NOT proof of lip-sync or face quality.
    A later window may not silently change frame geometry or pixel encoding.
    """
    if len(frames) != CHUNK_FRAMES:
        raise RuntimeError("real model did not produce ten BGR frames")
    shape = expected_shape
    for frame in frames:
        frame_shape = getattr(frame, "shape", None)
        if (getattr(frame, "ndim", None) != 3
                or not isinstance(frame_shape, tuple)
                or len(frame_shape) != 3
                or frame_shape[2] != 3
                or not all(isinstance(n, int) and n > 0 for n in frame_shape)
                or str(getattr(frame, "dtype", "")) != "uint8"):
            raise RuntimeError("MuseTalk CUDA output must be nonempty uint8 BGR")
        if shape is None:
            shape = frame_shape
        elif frame_shape != shape:
            raise RuntimeError("MuseTalk CUDA output frame dimensions changed")
    return shape


def approved_model_digest(root: str | Path) -> str:
    """Hash the exact model/config inputs before torch or pickle executes.

    The independent deployment pin is not proof of copyright or consent.
    Keep the approved model files immutable after the check to avoid TOCTOU.
    """
    base = Path(root).resolve(strict=True)
    if not base.is_dir():
        raise RuntimeError("MuseTalk model root missing")
    digest = hashlib.sha256(b"VPR-MuseTalk-runtime-weights-v1\\0")
    for name in REQUIRED_MODEL_FILES:
        file = base / name
        if file.is_symlink() or not file.is_file():
            raise RuntimeError(f"missing or symlinked MuseTalk model: {name}")
        size = file.stat().st_size
        if size == 0:
            raise RuntimeError(f"empty MuseTalk model: {name}")
        key = name.encode("utf-8")
        digest.update(len(key).to_bytes(4, "big"))
        digest.update(key)
        digest.update(size.to_bytes(8, "big"))
        with file.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
    return digest.hexdigest()


def validated_avatar_frame_paths(avatar_dir: str | Path) -> tuple[list[Path], list[Path]]:
    """Return aligned face/mask frames; reject ambiguous or mismatched sets.

    MuseTalk indexes both arrays with the same frame number. Equal counts
    alone do not make the mask for a different image safe to use.
    """
    avatar = Path(avatar_dir).resolve(strict=True)
    frames = []
    for directory in ("full_imgs", "mask"):
        folder = avatar / directory
        if folder.is_symlink() or not folder.is_dir():
            raise RuntimeError("prepared avatar frame directory unavailable")
        paths = sorted(folder.glob("*.png"))
        if not paths:
            raise RuntimeError("prepared avatar frames missing")
        if any(p.is_symlink() or not p.is_file() or p.stat().st_size == 0 for p in paths):
            raise RuntimeError("prepared avatar frames must be nonempty regular files")
        frames.append(paths)
    if [p.name for p in frames[0]] != [p.name for p in frames[1]]:
        raise RuntimeError("prepared avatar face and mask filenames differ")
    return frames[0], frames[1]


def prepared_avatar_digest(avatar_dir: str | Path) -> str:
    """Digest exactly the prepared face data consumed by this renderer.

    Caller pins the digest OUTSIDE the asset directory; a self-supplied
    manifest cannot authorize itself. Reject symlinks before pickle loading.
    No extra Persona owner, database or browser-provided asset selector.
    """
    avatar = Path(avatar_dir).resolve(strict=True)
    paths = [avatar / name for name in ("latents.pt", "coords.pkl", "mask_coords.pkl")]
    full_imgs, masks = validated_avatar_frame_paths(avatar)
    paths.extend(full_imgs)
    paths.extend(masks)
    digest = hashlib.sha256(b"VPR-MuseTalk-prepared-avatar-v1\\0")
    for path in sorted(paths, key=lambda candidate: candidate.relative_to(avatar).as_posix()):
        if path.is_symlink() or not path.is_file():
            raise RuntimeError("prepared avatar file must be a regular non-symlink")
        relative = path.relative_to(avatar).as_posix().encode("utf-8")
        # Name and byte count delimit file boundaries; content is streamed.
        digest.update(len(relative).to_bytes(4, "big"))
        digest.update(relative)
        digest.update(path.stat().st_size.to_bytes(8, "big"))
        with path.open("rb") as handle:
            for block in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(block)
    return digest.hexdigest()


def verify_upstream_source(root: str | Path, revision: str) -> None:
    """Enforce the reviewed MuseTalk checkout before loading any Python model.

    The offline preflight and the runtime use the same source-identity gate.
    A tracked source edit or injected untracked file must never get to import.
    Model weights are separately pinned; upstream models/ are gitignored.
    """
    if not isinstance(revision, str) or re.fullmatch(r"[0-9a-f]{40}", revision) is None:
        raise RuntimeError("RT1_MUSETALK_SOURCE_REVISION pin required")
    folder = Path(root).resolve(strict=True)
    def git(*args):
        try:
            return subprocess.check_output(
                ["git", "-C", str(folder), *args],
                text=True, stderr=subprocess.DEVNULL, timeout=10
            ).strip()
        except (OSError, subprocess.SubprocessError) as error:
            raise RuntimeError("MuseTalk source identity cannot be verified") from error
    if git("rev-parse", "HEAD") != revision:
        raise RuntimeError("MuseTalk source revision differs from approved commit")
    if git("status", "--porcelain", "--untracked-files=all"):
        raise RuntimeError("modified or untracked MuseTalk source files")


class MuseTalkRenderer:
    """One loaded model and one GPU inference lock for all owned RTP sessions."""

    def __init__(
        self, root: str, avatar_dir: str, *,
        persona_id: str, persona_version: int, gpu_id: int = 0,
    ):
        # This binding is supplied by an operator-controlled deployment and
        # checked against the canonical reviewed Persona on every admission.
        # Match vpr-domain::PersonaId, not a second ASCII-only policy.
        if (not isinstance(persona_id, str) or not persona_id.strip()
                or len(persona_id) > 256
                or type(persona_version) is not int or persona_version < 1):
            raise ValueError("invalid bound MuseTalk Persona identity")
        self._persona_id = persona_id
        self._persona_version = persona_version
        root_path = Path(root).resolve(strict=True)
        avatar = Path(avatar_dir).resolve(strict=True)
        if not root_path.is_dir() or not avatar.is_dir():
            raise ValueError("MuseTalk source and prepared avatar directories required")
        if Path.cwd().resolve() != root_path:
            raise RuntimeError("MuseTalk worker must start with cwd=RT1_MUSETALK_ROOT")
        if not (root_path / "musetalk" / "utils" / "utils.py").is_file():
            raise RuntimeError("MuseTalk source tree unavailable")
        required = (
            root_path / "models/musetalkV15/unet.pth",
            root_path / "models/musetalkV15/musetalk.json",
            root_path / "models/whisper/config.json",
            root_path / "models/whisper/preprocessor_config.json",
            root_path / "models/whisper/pytorch_model.bin",
            root_path / "models/sd-vae/config.json",
            root_path / "models/sd-vae/diffusion_pytorch_model.bin",
            avatar / "latents.pt",
            avatar / "coords.pkl",
            avatar / "mask_coords.pkl",
        )
        if any(not p.is_file() for p in required):
            raise RuntimeError("MuseTalk weights or prepared avatar assets missing")
        # Keep one frame-order contract for the offline preflight, digest and
        # CUDA renderer. The face and mask arrays must align by filename.
        full_imgs, masks = validated_avatar_frame_paths(avatar)
        # The operator-approved digest is an independent deployment secret/
        # configuration value, never computed from a visitor-supplied path.
        # Fail before importing CUDA or deserializing trusted-only pickle data.
        pinned = os.environ.get("RT1_MUSETALK_AVATAR_SHA256", "")
        if len(pinned) != 64 or any(ch not in "0123456789abcdef" for ch in pinned):
            raise RuntimeError("RT1_MUSETALK_AVATAR_SHA256 pin required")
        actual = prepared_avatar_digest(avatar)
        if not hmac.compare_digest(actual, pinned):
            raise RuntimeError("prepared Persona avatar digest mismatch")
        model_pin = os.environ.get("RT1_MUSETALK_MODELS_SHA256", "")
        if len(model_pin) != 64 or any(ch not in "0123456789abcdef" for ch in model_pin):
            raise RuntimeError("RT1_MUSETALK_MODELS_SHA256 pin required")
        if not hmac.compare_digest(approved_model_digest(root_path), model_pin):
            raise RuntimeError("approved MuseTalk model weights digest mismatch")
        verify_upstream_source(
            root_path, os.environ.get("RT1_MUSETALK_SOURCE_REVISION", "")
        )
        # An independently approved full installed-distribution fingerprint is
        # required before any GPU imports or deserialization of avatar assets.
        from rt1_musetalk_environment import verify_approved_environment
        verify_approved_environment(os.environ.get("RT1_MUSETALK_ENVIRONMENT_SHA256"))
        # Load only after local path, weights and dependency approval checks.
        import torch
        if not torch.cuda.is_available() or gpu_id >= torch.cuda.device_count() or gpu_id < 0:
            raise RuntimeError("CUDA GPU unavailable")
        sys.path.insert(0, str(root_path))
        from transformers import WhisperModel
        from musetalk.utils.utils import load_all_model
        from musetalk.utils.audio_processor import AudioProcessor
        from musetalk.utils.preprocessing import read_imgs
        import pickle

        device = torch.device(f"cuda:{gpu_id}")
        vae, unet, pe = load_all_model(
            unet_model_path=str(required[0]),
            unet_config=str(required[1]),
            device=device,
        )
        self._torch = torch
        self._device = device
        self._vae = vae
        self._unet = unet
        self._pe = pe.half().to(device).eval()
        vae.vae = vae.vae.half().to(device).eval()
        unet.model = unet.model.half().to(device).eval()
        whisper_dir = root_path / "models/whisper"
        self._whisper = WhisperModel.from_pretrained(
            str(whisper_dir), local_files_only=True
        ).to(device=device, dtype=unet.model.dtype).eval()
        self._whisper.requires_grad_(False)
        self._audio = AudioProcessor(feature_extractor_path=str(whisper_dir))
        self._latents = torch.load(
            str(avatar / "latents.pt"), map_location="cpu", weights_only=True
        )
        with (avatar / "coords.pkl").open("rb") as handle:
            self._coords = pickle.load(handle)
        with (avatar / "mask_coords.pkl").open("rb") as handle:
            self._mask_coords = pickle.load(handle)
        self._images = read_imgs([str(path) for path in full_imgs])
        self._masks = read_imgs([str(path) for path in masks])
        count = len(self._latents)
        if not count or any(len(x) != count for x in (
            self._coords, self._mask_coords, self._images, self._masks
        )):
            raise RuntimeError("MuseTalk prepared avatar frames are inconsistent")
        self._gpu_lock = threading.Lock()
        self._healthy = True
        self._timesteps = torch.tensor([0], device=device)
        self._prepare_sample = self._images[0].copy()
        if self._prepare_sample.ndim != 3 or self._prepare_sample.shape[2] != 3:
            raise RuntimeError("MuseTalk avatar must contain BGR frames")
        # Real model inference, not a file-presence or CUDA-only readiness
        # heuristic. Fail before serving HTTP if the prepared avatar cannot
        # generate valid frames on the selected GPU.
        import numpy as np
        warmup = self._infer(
            np.zeros(PCM_WINDOW_SAMPLES, dtype="<i2"), 0
        )
        if len(warmup) != CHUNK_FRAMES:
            raise RuntimeError("MuseTalk GPU model warmup produced no video")

    def persona_identity(self) -> tuple[str, int]:
        """Return the immutable, operator-approved Persona/asset binding."""
        return self._persona_id, self._persona_version

    def ready(self) -> bool:
        if not self._healthy:
            return False
        try:
            return self._torch.cuda.is_available() and bool(self._latents)
        except Exception:
            return False

    def create_video_track(self, operation_id: str, generation: int):
        if not self.ready():
            raise RuntimeError("MuseTalk renderer unavailable")
        # Never reuse the frame/PCM queues across visitor sessions.
        return _MuseTalkVideoTrack(self, operation_id, generation)

    def _infer(self, samples, start_frame: int):
        """Real Whisper -> positional encoding -> UNet -> VAE -> face blending."""
        import numpy as np
        import cv2
        from musetalk.utils.blending import get_image_blending
        from musetalk.utils.utils import datagen

        if len(samples) != PCM_WINDOW_SAMPLES:
            raise ValueError("MuseTalk inference requires exactly 400ms PCM")
        with self._gpu_lock, self._torch.inference_mode():
            features = self._audio.feature_extractor(
                np.asarray(samples, dtype=np.float32) / 32768.0,
                return_tensors="pt", sampling_rate=SAMPLE_RATE,
            ).input_features.to(self._unet.model.dtype)
            prompts = self._audio.get_whisper_chunk(
                [features], self._device, self._unet.model.dtype, self._whisper,
                len(samples), fps=25,
                audio_padding_length_left=2, audio_padding_length_right=2,
            )
            output = []
            # One inference pass processes this 400ms window; no image files
            # or browser commands are used in the live media path.
            for audio_batch, latent_batch in datagen(
                prompts,
                [self._latents[(start_frame + n) % len(self._latents)]
                 for n in range(len(prompts))],
                batch_size=CHUNK_FRAMES,
                device=self._device,
            ):
                audio_embed = self._pe(audio_batch.to(self._device))
                latent_batch = latent_batch.to(
                    device=self._device, dtype=self._unet.model.dtype
                )
                predicted = self._unet.model(
                    latent_batch, self._timesteps,
                    encoder_hidden_states=audio_embed,
                ).sample.to(device=self._device, dtype=self._vae.vae.dtype)
                faces = self._vae.decode_latents(predicted)
                for face in faces:
                    idx = (start_frame + len(output)) % len(self._latents)
                    bbox = self._coords[idx]
                    x1, y1, x2, y2 = map(int, bbox)
                    if x2 <= x1 or y2 <= y1:
                        raise RuntimeError("MuseTalk invalid prepared face bounds")
                    crop = cv2.resize(
                        face.astype(np.uint8), (x2 - x1, y2 - y1)
                    )
                    composed = get_image_blending(
                        self._images[idx].copy(), crop, bbox,
                        self._masks[idx], self._mask_coords[idx],
                    )
                    if composed is None or composed.ndim != 3:
                        raise RuntimeError("MuseTalk GPU generated invalid frame")
                    output.append(composed)
            # Validate the same contract at every production GPU inference,
            # not only while writing an offline hardware acceptance report.
            validate_frame_batch(output)
            return output


class _TrackTransportBackpressure(RuntimeError):
    """Session-local RTP/queue failure, not a shared CUDA model failure."""


class _MuseTalkVideoTrack:
    """Created as an aiortc.VideoStreamTrack subclass at runtime.

    Heavy model inference runs in exactly one bounded producer per track;
    the aiortc event loop never executes CUDA kernels.
    """

    def __new__(cls, renderer: MuseTalkRenderer, operation_id: str, generation: int):
        from aiortc import VideoStreamTrack
        from av import VideoFrame
        import numpy as np

        class Track(VideoStreamTrack):
            def __init__(self):
                super().__init__()
                self._renderer = renderer
                self._pcm = bytearray()
                self._chunks = queue.Queue(maxsize=MAX_AUDIO_CHUNKS)
                self._frames = queue.Queue(maxsize=MAX_VIDEO_FRAMES)
                self._lock = threading.Lock()
                self._stop = threading.Event()
                self._failed = None
                self._frame_index = 0
                self._audio_track = None
                self._current = renderer._prepare_sample.copy()
                self._producer = threading.Thread(
                    target=self._process, name="musetalk-gpu-frame",
                    daemon=True,
                )
                self._producer.start()

            def bind_audio_track(self, audio):
                # Before the sender publishes any PCM, bind the single
                # existing RTP audio track. Only model-rendered video frames
                # can release their matching audio chunk into that sender.
                with self._lock:
                    if self._audio_track is not None:
                        raise RuntimeError("MuseTalk audio sender already bound")
                    self._audio_track = audio

            def push_pcm(self, pcm: bytes, *, final: bool = False):
                if not pcm or len(pcm) % 640 or type(final) is not bool:
                    raise ValueError("PCM must contain whole 20ms frames and a valid final marker")
                with self._lock:
                    if self._stop.is_set() or self.readyState != "live":
                        raise RuntimeError("Persona video sender stopped")
                    if self._failed is not None:
                        raise RuntimeError("MuseTalk renderer failed") from self._failed
                    # No unbounded buffering and no silent discarded speech.
                    whole = bytes(self._pcm) + pcm
                    if final and len(whole) % PCM_WINDOW_BYTES:
                        # An explicit utterance boundary is the ONLY safe
                        # time to pad. A timer would inject silence between
                        # the canonical provider's paced streaming chunks.
                        whole += bytes(PCM_WINDOW_BYTES - len(whole) % PCM_WINDOW_BYTES)
                    chunks = len(whole) // PCM_WINDOW_BYTES
                    if self._chunks.qsize() + chunks > MAX_AUDIO_CHUNKS:
                        raise RuntimeError("MuseTalk PCM backlog exceeded")
                    self._pcm = bytearray(whole[chunks * PCM_WINDOW_BYTES:])
                    for start in range(0, chunks * PCM_WINDOW_BYTES, PCM_WINDOW_BYTES):
                        self._chunks.put_nowait(whole[start:start + PCM_WINDOW_BYTES])

            def _process(self):
                import numpy as np
                try:
                    while not self._stop.is_set():
                        try:
                            payload = self._chunks.get(timeout=0.1)
                        except queue.Empty:
                            continue
                        pcm = np.frombuffer(payload, dtype="<i2")
                        frames = renderer._infer(pcm, self._frame_index)
                        self._frame_index += len(frames)
                        for frame in frames:
                            if self._stop.is_set():
                                return
                            try:
                                self._frames.put(frame, timeout=0.2)
                            except queue.Full:
                                raise _TrackTransportBackpressure("MuseTalk frame backlog exceeded")
                        if self._stop.is_set():
                            return
                        # Hold audio until its real inferred lip-sync frames
                        # have been queued; audio/video now become available
                        # to the EXISTING aiortc senders together.
                        audio = self._audio_track
                        if audio is None:
                            # A missing binding is a bad session allocation,
                            # not evidence of a failed shared CUDA model.
                            raise _TrackTransportBackpressure(
                                "MuseTalk session audio sender not bound"
                            )
                        try:
                            audio.push_pcm(payload)
                        except Exception as error:
                            # The sender is an independently owned transport.
                            # Regardless of its local exception class, it must
                            # not poison the shared GPU model for other visitors.
                            raise _TrackTransportBackpressure(
                                "MuseTalk session audio sender rejected PCM"
                            ) from error
                except Exception as error:
                    self._failed = error
                    if isinstance(error, _TrackTransportBackpressure):
                        LOG.warning(
                            "MuseTalk session transport failed; canonical "
                            "STOP reconciliation required", exc_info=True
                        )
                    else:
                        renderer._healthy = False
                        LOG.exception("MuseTalk model inference failed; renderer not ready")

            def render_failed(self) -> bool:
                """Expose asynchronous GPU/transport failures to the ONE ledger owner."""
                return self._failed is not None

            async def next_timestamp(self):
                """Pace *model-generated* frames at 25 fps, not aiortc's 30.

                RTP's 90 kHz video clock advances 3,600 ticks per frame.
                Match MuseTalk's 40 ms PCM/frame and never accelerate video
                to aiortc's 30 fps diagnostic VideoStreamTrack default.
                """
                from aiortc.mediastreams import MediaStreamError
                if self.readyState != "live" or self._stop.is_set():
                    raise MediaStreamError
                if not hasattr(self, "_timestamp"):
                    self._start = time.monotonic()
                    self._timestamp = 0
                else:
                    self._timestamp += VIDEO_FRAME_PTS
                    deadline = self._start + self._timestamp / VIDEO_RTP_CLOCK_HZ
                    await asyncio.sleep(max(0.0, deadline - time.monotonic()))
                return self._timestamp, VIDEO_TIME_BASE

            async def recv(self):
                from aiortc.mediastreams import MediaStreamError
                if self.readyState != "live" or self._stop.is_set():
                    raise MediaStreamError
                if self._failed is not None or not renderer.ready():
                    raise MediaStreamError
                pts, time_base = await self.next_timestamp()
                # STOP can arrive while pacing the next 25 fps frame.
                # Never release an already-revoked image after that await.
                if self.readyState != "live" or self._stop.is_set():
                    raise MediaStreamError
                try:
                    self._current = self._frames.get_nowait()
                except queue.Empty:
                    # A real prepared avatar frame, never a synthetic face.
                    pass
                frame = VideoFrame.from_ndarray(
                    np.ascontiguousarray(self._current), format="bgr24"
                )
                frame.pts, frame.time_base = pts, time_base
                return frame

            def stop(self):
                self._stop.set()
                with self._lock:
                    self._pcm.clear()
                super().stop()

            def wait_quiesced(self, timeout: float) -> bool:
                """Only the canonical sender STOP may await this GPU fence."""
                self._producer.join(timeout=max(0.0, timeout))
                return not self._producer.is_alive()

        return Track()


def renderer_from_environment():
    """Explicit opt-in only. Failure is fatal; no diagnostic fallback."""
    choice = os.environ.get("RT1_VIDEO_RENDERER", "").strip()
    if not choice:
        return None
    if choice != "musetalk-v15":
        raise ValueError("unsupported RT1_VIDEO_RENDERER configuration")
    root = os.environ["RT1_MUSETALK_ROOT"]
    avatar = os.environ["RT1_MUSETALK_AVATAR_DIR"]
    persona_id = os.environ["RT1_MUSETALK_PERSONA_ID"]
    persona_version = int(os.environ["RT1_MUSETALK_PERSONA_VERSION"])
    return MuseTalkRenderer(
        root, avatar, persona_id=persona_id,
        persona_version=persona_version,
        gpu_id=int(os.environ.get("RT1_MUSETALK_GPU_ID", "0"))
    )
