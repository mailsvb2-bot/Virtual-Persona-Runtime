"""GPU-backed MuseTalk v1.5 Persona video, driven only by canonical 16k PCM.

The aiortc sender still owns CREATE/SDP/RTP/STOP; this module only renders
frames. No model downloads, fallback faces, policy, TTS or second media owner.
Requires an operator-prepared MuseTalk avatar and local CUDA model weights.
"""
from __future__ import annotations

import asyncio
from fractions import Fraction
import logging
import os
from pathlib import Path
import queue
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


class MuseTalkRenderer:
    """One loaded model and one GPU inference lock for all owned RTP sessions."""

    def __init__(self, root: str, avatar_dir: str, *, gpu_id: int = 0):
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
            avatar / "latents.pt",
            avatar / "coords.pkl",
            avatar / "mask_coords.pkl",
        )
        if any(not p.is_file() for p in required):
            raise RuntimeError("MuseTalk weights or prepared avatar assets missing")
        if not list((avatar / "full_imgs").glob("*.png")):
            raise RuntimeError("MuseTalk avatar image frames missing")
        if not list((avatar / "mask").glob("*.png")):
            raise RuntimeError("MuseTalk avatar masks missing")
        # Load only after local path, weights and CUDA checks. No implicit downloads.
        import torch
        if not torch.cuda.is_available() or gpu_id >= torch.cuda.device_count() or gpu_id < 0:
            raise RuntimeError("CUDA GPU unavailable")
        sys.path.insert(0, str(root_path))
        from transformers import WhisperModel
        from musetalk.utils.utils import load_all_model
        from musetalk.utils.audio_processor import AudioProcessor
        from musetalk.utils.preprocessing import read_imgs
        import glob
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
        self._images = read_imgs(sorted(glob.glob(str(avatar / "full_imgs/*.png"))))
        self._masks = read_imgs(sorted(glob.glob(str(avatar / "mask/*.png"))))
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
            if len(output) != CHUNK_FRAMES:
                raise RuntimeError("MuseTalk generated incomplete PCM window")
            return output


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
                                raise RuntimeError("MuseTalk frame backlog exceeded")
                        if self._stop.is_set():
                            return
                        # Hold audio until its real inferred lip-sync frames
                        # have been queued; audio/video now become available
                        # to the EXISTING aiortc senders together.
                        audio = self._audio_track
                        if audio is None:
                            raise RuntimeError("MuseTalk audio sender not bound")
                        audio.push_pcm(payload)
                except Exception as error:
                    self._failed = error
                    renderer._healthy = False
                    LOG.exception("MuseTalk inference failed; renderer not ready")

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
    return MuseTalkRenderer(
        root, avatar, gpu_id=int(os.environ.get("RT1_MUSETALK_GPU_ID", "0"))
    )
