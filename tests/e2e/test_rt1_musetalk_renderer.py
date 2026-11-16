"""Hermetic contract for the opt-in real MuseTalk renderer composition root.

GPU inference and live A/V timing require a separately measured CUDA host.
No synthetic renderer is allowed to authorize production publication.
"""
import os
from pathlib import Path
import sys
import tempfile
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
            "RT1_MUSETALK_GPU_ID": "2",
        }, clear=True), patch.object(model, "MuseTalkRenderer") as renderer:
            result = model.renderer_from_environment()
            renderer.assert_called_once_with(
                "/srv/musetalk", "/srv/avatar/persona", gpu_id=2
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
                    model.MuseTalkRenderer(str(source), str(avatar))

    def test_track_uses_exact_authorized_pcm_window_size(self):
        self.assertEqual(model.SAMPLE_RATE, 16000)
        self.assertEqual(model.SAMPLES_PER_FRAME, 640)
        self.assertEqual(model.CHUNK_FRAMES, 10)
        self.assertEqual(model.CHUNK_FRAMES * model.SAMPLES_PER_FRAME, 6400)
        self.assertLessEqual(model.MAX_AUDIO_CHUNKS, 4)
        self.assertLessEqual(model.MAX_VIDEO_FRAMES, 25)


if __name__ == "__main__":
    unittest.main()
