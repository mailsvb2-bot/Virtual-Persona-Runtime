"""Fail-closed prepared Persona asset provenance guard (no CUDA dependency)."""
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from rt1_musetalk_renderer import MuseTalkRenderer, prepared_avatar_digest


def prepare(directory: Path):
    (directory / "full_imgs").mkdir()
    (directory / "mask").mkdir()
    for name in ("latents.pt", "coords.pkl", "mask_coords.pkl",
                 "full_imgs/0001.png", "mask/0001.png"):
        (directory / name).write_bytes(("authorized:" + name).encode())


class PreparedAvatarDigestTest(unittest.TestCase):
    def test_digest_is_deterministic_and_detects_modified_face(self):
        with tempfile.TemporaryDirectory() as temp:
            avatar = Path(temp)
            prepare(avatar)
            pinned = prepared_avatar_digest(avatar)
            self.assertEqual(len(pinned), 64)
            self.assertEqual(pinned, prepared_avatar_digest(avatar))
            (avatar / "full_imgs/0001.png").write_bytes(b"replaced")
            self.assertNotEqual(pinned, prepared_avatar_digest(avatar))

    def test_digest_detects_changed_pickle_and_added_frame(self):
        with tempfile.TemporaryDirectory() as temp:
            avatar = Path(temp)
            prepare(avatar)
            old = prepared_avatar_digest(avatar)
            (avatar / "coords.pkl").write_bytes(b"untrusted replacement")
            self.assertNotEqual(old, prepared_avatar_digest(avatar))
            changed = prepared_avatar_digest(avatar)
            (avatar / "mask/0002.png").write_bytes(b"unexpected frame")
            with self.assertRaisesRegex(RuntimeError, "filenames differ"):
                prepared_avatar_digest(avatar)

    def test_rejects_same_count_mismatched_face_and_mask_names(self):
        with tempfile.TemporaryDirectory() as temp:
            avatar = Path(temp)
            prepare(avatar)
            (avatar / "mask/0001.png").rename(avatar / "mask/0002.png")
            with self.assertRaisesRegex(RuntimeError, "filenames differ"):
                prepared_avatar_digest(avatar)

    def test_rejects_empty_face_or_mask_before_hashing(self):
        with tempfile.TemporaryDirectory() as temp:
            avatar = Path(temp)
            prepare(avatar)
            (avatar / "mask/0001.png").write_bytes(b"")
            with self.assertRaisesRegex(RuntimeError, "nonempty regular files"):
                prepared_avatar_digest(avatar)

    def test_rejects_symlinked_prepared_files(self):
        with tempfile.TemporaryDirectory() as temp:
            avatar = Path(temp)
            prepare(avatar)
            path = avatar / "coords.pkl"
            target = avatar / "outside.bin"
            target.write_bytes(b"not approved")
            path.unlink()
            try:
                path.symlink_to(target)
            except (OSError, NotImplementedError):
                self.skipTest("filesystem does not support symlinks")
            with self.assertRaisesRegex(RuntimeError, "non-symlink"):
                prepared_avatar_digest(avatar)

    def test_missing_pin_fails_before_importing_cuda_or_loading_pickle(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "upstream"
            avatar = Path(temp) / "prepared"
            (root / "musetalk/utils").mkdir(parents=True)
            (root / "musetalk/utils/utils.py").touch()
            (root / "models/musetalkV15").mkdir(parents=True)
            (root / "models/whisper").mkdir(parents=True)
            (root / "models/sd-vae").mkdir(parents=True)
            for name in ("models/musetalkV15/unet.pth",
                         "models/musetalkV15/musetalk.json",
                         "models/whisper/config.json",
                         "models/whisper/preprocessor_config.json",
                         "models/whisper/pytorch_model.bin",
                         "models/sd-vae/config.json",
                         "models/sd-vae/diffusion_pytorch_model.bin"):
                (root / name).touch()
            avatar.mkdir()
            prepare(avatar)
            with patch("pathlib.Path.cwd", return_value=root), patch.dict(
                    os.environ, {}, clear=True):
                with self.assertRaisesRegex(RuntimeError, "SHA256 pin required"):
                    MuseTalkRenderer(str(root), str(avatar),
                                     persona_id="persona-a", persona_version=1)
                with patch.dict(os.environ, {
                    "RT1_MUSETALK_AVATAR_SHA256": "0" * 64,
                }):
                    with self.assertRaisesRegex(RuntimeError, "digest mismatch"):
                        MuseTalkRenderer(str(root), str(avatar),
                                         persona_id="persona-a", persona_version=1)


if __name__ == "__main__":
    unittest.main()
