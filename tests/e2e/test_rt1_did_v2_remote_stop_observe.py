"""Fail-closed contract for real D-ID V2 remote STOP observer."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch
import io
from contextlib import redirect_stdout, redirect_stderr

PATH = Path(__file__).resolve().parents[2] / "scripts" / "rt1_did_v2_remote_stop_observe.py"
spec = importlib.util.spec_from_file_location("did_stop_probe", PATH)
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class ObserverTests(unittest.TestCase):
    def test_no_credentials_cannot_run(self):
        with patch.dict("os.environ", {"VPR_DID_API_KEY": ""}):
            with patch("sys.argv", ["probe", "--session-id", "id"]):
                with redirect_stderr(io.StringIO()):
                    self.assertEqual(probe.main(), 2)

    def test_provider_status_is_bounded_to_nonsecret_category(self):
        self.assertEqual(probe.status_category({"status": "active"}), "active")
        self.assertEqual(probe.status_category({"status": "closed"}), "terminal")
        self.assertEqual(probe.status_category({"status": "Bearer TOP_SECRET"}), "other")
        self.assertEqual(probe.status_category({"status": {"token": "TOP_SECRET"}}), "missing")

    def test_provider_telemetry_never_promotes_remote_stop(self):
        private = {"session_token": "TOP_SECRET", "participants": [{"email": "private@example.com"}],
                   "status": "Bearer TOP_SECRET", "bandwidth": {"private": "value"}}
        with patch.dict("os.environ", {"VPR_DID_API_KEY": "test-key"}):
            with patch("sys.argv", ["probe", "--session-id", "test-session", "--samples", "2", "--interval", "1"]):
                with patch.object(probe, "read_session", return_value=(200, private)):
                    with patch.object(probe.time, "sleep"):
                        stdout, stderr = io.StringIO(), io.StringIO()
                        with redirect_stdout(stdout), redirect_stderr(stderr):
                            self.assertEqual(probe.main(), 3)
                        self.assertNotIn("TOP_SECRET", stdout.getvalue())
                        self.assertIn('"provider_status_category": "other"', stdout.getvalue())
                        self.assertNotIn("private@example.com", stdout.getvalue())
                        self.assertNotIn("bandwidth", stdout.getvalue())
                        self.assertIn("REMOTE_VIDEO_STOP_UNCONFIRMED", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
