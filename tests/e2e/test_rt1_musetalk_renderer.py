"""Hermetic contract for the opt-in real MuseTalk renderer composition root.

GPU inference and live A/V timing require a separately measured CUDA host.
No synthetic renderer is allowed to authorize production publication.
"""
import asyncio
from fractions import Fraction
import os
from pathlib import Path
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import rt1_musetalk_renderer as model


class MuseTalkFactoryTest(unittest.TestCase):
    def test_not_configured_never_creates_renderer(self):
        with patch.dict(os.environ, {}, clear=True):
            self.assertIsNone(model.renderer_from_environment())

    def test_unknown_renderer_never_falls_back_to_diagnostic(self):
        with patch.dict(os.environ, {"RT1_VIDEO_RENDERER": "unknown-model"}, clear=True):
            with self.assertRaisesRegex(ValueError, "unsupported"):
                model.renderer_from_environment()

    def test_misconfigured_model_requires_paths(self):
        with patch.dict(os.environ, {"RT1_VIDEO_RENDERER": "musetalk-v15"}, clear=True):
            with self.assertRaises(KeyError):
                model.renderer_from_environment()

    def test_opt_in_receives_explicit_model_asset_paths(self):
        with patch.dict(os.environ, {
            "RT1_VIDEO_RENDERER": "musetalk-v15",
            "RT1_MUSETALK_ROOT": "/srv/musetalk",
            "RT1_MUSETALK_AVATAR_DIR": "/srv/avatar/persona",
            "RT1_MUSETALK_PERSONA_ID": "persona-a",
            "RT1_MUSETALK_PERSONA_VERSION": "3",
            "RT1_MUSETALK_GPU_ID": "2",
        }, clear=True), patch.object(model, "MuseTalkRenderer") as renderer:
            result = model.renderer_from_environment()
            renderer.assert_called_once_with(
                "/srv/musetalk", "/srv/avatar/persona",
                persona_id="persona-a", persona_version=3, gpu_id=2
            )
            self.assertIs(result, renderer.return_value)

    def test_local_source_and_prepared_assets_required_before_importing_cuda(self):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "musetalk"
            avatar = Path(temp) / "avatar"
            source.mkdir()
            avatar.mkdir()
            with patch("pathlib.Path.cwd", return_value=source):
                with self.assertRaisesRegex(RuntimeError, "source tree unavailable"):
                    model.MuseTalkRenderer(
                        str(source), str(avatar), persona_id="persona-a",
                        persona_version=3,
                    )

    def test_bound_identity_validation_fails_before_gpu_loading(self):
        with tempfile.TemporaryDirectory() as base:
            for persona_id, version in (
                ("", 1), (" ", 1), ("Ж" * 257, 1),
                ("persona-a", 0), ("persona-a", True),
            ):
                with self.subTest(persona_id=persona_id, version=version):
                    with self.assertRaisesRegex(ValueError, "identity"):
                        model.MuseTalkRenderer(
                            base, base, persona_id=persona_id,
                            persona_version=version,
                        )

    def test_unicode_reviewed_persona_identity_is_accepted_before_asset_probe(self):
        with tempfile.TemporaryDirectory() as temp:
            source, avatar = Path(temp) / "musetalk", Path(temp) / "avatar"
            source.mkdir()
            avatar.mkdir()
            with patch("pathlib.Path.cwd", return_value=source):
                with self.assertRaisesRegex(RuntimeError, "source tree unavailable"):
                    model.MuseTalkRenderer(
                        str(source), str(avatar),
                        persona_id="Говорящая персона", persona_version=2,
                    )

    def test_track_uses_exact_authorized_pcm_window_size(self):
        self.assertEqual(model.SAMPLE_RATE, 16000)
        self.assertEqual(model.SAMPLES_PER_FRAME, 640)
        self.assertEqual(model.CHUNK_FRAMES, 10)
        self.assertEqual(model.CHUNK_FRAMES * model.SAMPLES_PER_FRAME, 6400)
        self.assertEqual(model.PCM_WINDOW_SAMPLES, 6400)
        self.assertEqual(model.PCM_WINDOW_BYTES, 12800)
        self.assertLessEqual(model.MAX_AUDIO_CHUNKS, 4)
        self.assertLessEqual(model.MAX_VIDEO_FRAMES, 25)


class MuseTalkRtpTimingTest(unittest.IsolatedAsyncioTestCase):
    async def test_inferred_video_uses_25_fps_rtp_clock_not_aiortc_default(self):
        import numpy as np

        class PreparedAvatar:
            _prepare_sample = np.zeros((48, 64, 3), dtype=np.uint8)

            def ready(self):
                return True

            def _infer(self, samples, frame_index):
                self.fail("no inference is needed for idle track timestamps")

        avatar = PreparedAvatar()
        track = model._MuseTalkVideoTrack(avatar, "timing", 1)
        try:
            first = await track.recv()
            second = await track.recv()
            self.assertEqual((first.pts, second.pts), (0, 3600))
            self.assertEqual(first.time_base, Fraction(1, 90000))
            self.assertEqual(second.time_base, first.time_base)
            self.assertEqual(model.VIDEO_FPS, 25)
            self.assertEqual(model.VIDEO_FRAME_PTS, 3600)
            self.assertEqual(model.SAMPLES_PER_FRAME * model.VIDEO_FPS,
                             model.SAMPLE_RATE)
        finally:
            track.stop()
            self.assertTrue(track.wait_quiesced(1.0))


class MuseTalkPcmWindowTest(unittest.TestCase):
    def _track(self):
        import numpy as np

        class FakeGpu:
            _prepare_sample = np.zeros((48, 64, 3), dtype=np.uint8)

            def __init__(self):
                self.sample_lengths = []

            def ready(self):
                return True

            def _infer(self, samples, start_frame):
                self.sample_lengths.append(len(samples))
                if len(samples) != model.PCM_WINDOW_SAMPLES:
                    raise ValueError("incorrect PCM window")
                return [self._prepare_sample.copy() for _ in range(model.CHUNK_FRAMES)]

        class Audio:
            def __init__(self):
                self.received = []
                self.sent = threading.Event()

            def push_pcm(self, pcm):
                self.received.append(pcm)
                self.sent.set()

        gpu = FakeGpu()
        audio = Audio()
        track = model._MuseTalkVideoTrack(gpu, "pcm-window", 1)
        track.bind_audio_track(audio)
        return gpu, audio, track

    def test_400ms_window_is_12800_bytes_not_6400(self):
        gpu, audio, track = self._track()
        try:
            pcm = bytes([1, 0]) * model.PCM_WINDOW_SAMPLES
            self.assertEqual(len(pcm), 12_800)
            track.push_pcm(pcm)
            self.assertTrue(audio.sent.wait(2.0))
            self.assertEqual(gpu.sample_lengths, [6400])
            self.assertEqual(audio.received, [pcm])
        finally:
            track.stop()
            self.assertTrue(track.wait_quiesced(2.0))

    def test_final_short_fragment_is_padded_only_at_utterance_end(self):
        gpu, audio, track = self._track()
        try:
            first = bytes([1, 0]) * 3200  # 200ms, not a full MuseTalk window
            last = bytes([2, 0]) * 320     # 20ms: canonical final marker
            track.push_pcm(first)
            self.assertFalse(audio.sent.wait(0.05))
            track.push_pcm(last, final=True)
            self.assertTrue(audio.sent.wait(2.0))
            self.assertEqual(gpu.sample_lengths, [6400])
            expected = first + last
            self.assertEqual(audio.received, [
                expected + bytes(model.PCM_WINDOW_BYTES - len(expected))
            ])
        finally:
            track.stop()
            self.assertTrue(track.wait_quiesced(2.0))


    def test_one_rtp_audio_failure_does_not_poison_shared_cuda_model(self):
        gpu, audio, track = self._track()
        gpu._healthy = True
        def reject_pcm(_pcm):
            raise RuntimeError("remote audio sender backpressure")
        audio.push_pcm = reject_pcm
        try:
            track.push_pcm(bytes(12800))
            for _ in range(100):
                if track.render_failed():
                    break
                time.sleep(0.01)
            self.assertTrue(track.render_failed())
            self.assertTrue(gpu._healthy)
        finally:
            track.stop()
            self.assertTrue(track.wait_quiesced(2.0))

    def test_missing_session_audio_binding_does_not_poison_shared_gpu(self):
        gpu, audio, track = self._track()
        gpu._healthy = True
        # Deliberately undo the session binding. A malformed allocation must
        # fail this track only, never disable other visitors' shared model.
        track._audio_track = None
        try:
            track.push_pcm(bytes(12800))
            for _ in range(100):
                if track.render_failed():
                    break
                time.sleep(0.01)
            self.assertTrue(track.render_failed())
            self.assertTrue(gpu._healthy)
        finally:
            track.stop()
            self.assertTrue(track.wait_quiesced(2.0))

    def test_non_runtime_audio_sender_failure_is_session_local(self):
        gpu, audio, track = self._track()
        gpu._healthy = True
        def reject_pcm(_pcm):
            raise ValueError("session-local RTP encoder error")
        audio.push_pcm = reject_pcm
        try:
            track.push_pcm(bytes(12800))
            for _ in range(100):
                if track.render_failed():
                    break
                time.sleep(0.01)
            self.assertTrue(track.render_failed())
            self.assertTrue(gpu._healthy)
        finally:
            track.stop()
            self.assertTrue(track.wait_quiesced(2.0))

    def test_actual_inference_exception_poison_shared_model(self):
        gpu, audio, track = self._track()
        gpu._healthy = True
        def broken_model(_samples, _frame):
            raise RuntimeError("CUDA inference failed")
        gpu._infer = broken_model
        try:
            track.push_pcm(bytes(12800))
            for _ in range(100):
                if track.render_failed():
                    break
                time.sleep(0.01)
            self.assertTrue(track.render_failed())
            self.assertFalse(gpu._healthy)
        finally:
            track.stop()
            self.assertTrue(track.wait_quiesced(2.0))


if __name__ == "__main__":
    unittest.main()
