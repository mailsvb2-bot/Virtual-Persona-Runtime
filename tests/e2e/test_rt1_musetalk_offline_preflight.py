"""Offline preflight contract: no CUDA, no pickle and no false release success."""
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from rt1_musetalk_renderer import prepared_avatar_digest
import rt1_musetalk_offline_preflight as preflight

REV = "a" * 40


class OfflinePreflightTest(unittest.TestCase):
    def fixture(self, base):
        root, avatar = Path(base) / "upstream", Path(base) / "avatar"
        root.mkdir()
        avatar.mkdir()
        sources = {
            "musetalk/utils/utils.py": (
                "def load_all_model(unet_model_path, unet_config, device): pass\n"
                "def datagen(whisper_chunks, vae_encode_latents, batch_size, device): pass\n"
            ),
            "musetalk/utils/audio_processor.py": (
                "class AudioProcessor:\n"
                " def get_whisper_chunk(self, whisper_input_features, device, weight_dtype,"
                " whisper, librosa_length, fps, audio_padding_length_left,"
                " audio_padding_length_right): pass\n"
            ),
            "musetalk/utils/blending.py": (
                "def get_image_blending(ori_frame, res_frame, bbox, mask, mask_crop_box): pass\n"
            ),
        }
        for name, source in sources.items():
            file = root / name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text(source)
        for name in preflight.MODEL_FILES:
            file = root / name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes(b"approved-model")
        for name in ("latents.pt", "coords.pkl", "mask_coords.pkl",
                     "full_imgs/0001.png", "mask/0001.png"):
            file = avatar / name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes(name.encode())
        return root, avatar

    def test_matching_source_and_assets_do_not_claim_gpu_pass(self):
        with tempfile.TemporaryDirectory() as temp:
            root, avatar = self.fixture(temp)
            with patch("subprocess.check_output", side_effect=[REV, ""]):
                result = preflight.inspect(
                    root, avatar, revision=REV,
                    digest=prepared_avatar_digest(avatar)
                )
            self.assertTrue(result["upstream_apis_checked"])
            self.assertFalse(result["cuda_executed"])
            self.assertFalse(result["real_gpu_accepted"])
            self.assertFalse(result["production_release_approved"])

    def test_revision_mismatch_fails(self):
        with tempfile.TemporaryDirectory() as temp:
            root, avatar = self.fixture(temp)
            with patch("subprocess.check_output", return_value="b" * 40):
                with self.assertRaisesRegex(ValueError, "revision differs"):
                    preflight.inspect(root, avatar, revision=REV,
                                      digest=prepared_avatar_digest(avatar))

    def test_untracked_source_injection_fails_before_model_load(self):
        with tempfile.TemporaryDirectory() as temp:
            root, avatar = self.fixture(temp)
            with patch("subprocess.check_output", side_effect=[
                REV, "?? musetalk/utils/override.py"
            ]) as git_call:
                with self.assertRaisesRegex(ValueError, "untracked MuseTalk source"):
                    preflight.inspect(root, avatar, revision=REV,
                                      digest=prepared_avatar_digest(avatar))
            self.assertIn("--untracked-files=all",
                          git_call.call_args_list[1].args[0])

    def test_api_drift_fails_before_cuda(self):
        with tempfile.TemporaryDirectory() as temp:
            root, _ = self.fixture(temp)
            (root / "musetalk/utils/audio_processor.py").write_text(
                "class AudioProcessor:\n def get_whisper_chunk(self, broken): pass\n"
            )
            with self.assertRaisesRegex(ValueError, "incompatible upstream API"):
                preflight.check_apis(root)

    def test_changed_face_fails_digest(self):
        with tempfile.TemporaryDirectory() as temp:
            root, avatar = self.fixture(temp)
            with patch("subprocess.check_output", side_effect=[REV, ""]):
                with self.assertRaisesRegex(ValueError, "approved digest"):
                    preflight.inspect(root, avatar, revision=REV, digest="0" * 64)

    def test_missing_vae_fails(self):
        with tempfile.TemporaryDirectory() as temp:
            root, avatar = self.fixture(temp)
            (root / "models/sd-vae/config.json").unlink()
            with patch("subprocess.check_output", side_effect=[REV, ""]):
                with self.assertRaisesRegex(ValueError, "MuseTalk model"):
                    preflight.inspect(root, avatar, revision=REV,
                                      digest=prepared_avatar_digest(avatar))


if __name__ == "__main__":
    unittest.main()
