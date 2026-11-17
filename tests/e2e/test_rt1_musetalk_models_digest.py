"""Zero-CUDA provenance checks for all runtime MuseTalk model bytes."""
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from rt1_musetalk_renderer import (
    MuseTalkRenderer, REQUIRED_MODEL_FILES, approved_model_digest,
    prepared_avatar_digest,
)


def stage(root, avatar):
    (root / "musetalk/utils").mkdir(parents=True)
    (root / "musetalk/utils/utils.py").write_text("# trusted fixture")
    for name in REQUIRED_MODEL_FILES:
        file = root / name
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_bytes(("approved:" + name).encode())
    for name in ("latents.pt", "coords.pkl", "mask_coords.pkl",
                 "full_imgs/0001.png", "mask/0001.png"):
        file = avatar / name
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_bytes(("approved:" + name).encode())


class ModelWeightsPinContract(unittest.TestCase):
    def test_digest_is_stable_and_detects_weight_replacement(self):
        with tempfile.TemporaryDirectory() as temp:
            root, avatar = Path(temp) / "root", Path(temp) / "avatar"
            stage(root, avatar)
            original = approved_model_digest(root)
            self.assertEqual(len(original), 64)
            self.assertEqual(original, approved_model_digest(root))
            (root / "models/musetalkV15/unet.pth").write_bytes(b"replaced")
            self.assertNotEqual(original, approved_model_digest(root))

    def test_worker_rejects_missing_or_wrong_model_pin_before_torch(self):
        with tempfile.TemporaryDirectory() as temp:
            root, avatar = Path(temp) / "root", Path(temp) / "avatar"
            stage(root, avatar)
            base_env = {
                "RT1_MUSETALK_AVATAR_SHA256": prepared_avatar_digest(avatar),
            }
            with patch("pathlib.Path.cwd", return_value=root), patch.dict(
                os.environ, base_env, clear=True
            ):
                with self.assertRaisesRegex(RuntimeError, "MODELS_SHA256 pin required"):
                    MuseTalkRenderer(str(root), str(avatar),
                                     persona_id="persona-1", persona_version=1)
                os.environ["RT1_MUSETALK_MODELS_SHA256"] = "0" * 64
                with self.assertRaisesRegex(RuntimeError, "weights digest mismatch"):
                    MuseTalkRenderer(str(root), str(avatar),
                                     persona_id="persona-1", persona_version=1)

    def test_unapproved_symlinked_model_is_not_hashed(self):
        with tempfile.TemporaryDirectory() as temp:
            root, avatar = Path(temp) / "root", Path(temp) / "avatar"
            stage(root, avatar)
            weight = root / "models/whisper/pytorch_model.bin"
            replacement = root / "elsewhere"
            replacement.write_bytes(b"bad")
            weight.unlink()
            try:
                weight.symlink_to(replacement)
            except (OSError, NotImplementedError):
                self.skipTest("symlinks unavailable")
            with self.assertRaisesRegex(RuntimeError, "symlinked MuseTalk model"):
                approved_model_digest(root)


if __name__ == "__main__":
    unittest.main()
