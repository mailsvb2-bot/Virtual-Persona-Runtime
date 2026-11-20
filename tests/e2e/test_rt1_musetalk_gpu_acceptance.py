"""GPU acceptance evaluation is hermetic; live run is intentionally opt-in."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from rt1_musetalk_gpu_acceptance import evaluate_windows, validate_frame_batch


class GpuAcceptanceEvidenceContract(unittest.TestCase):
    def test_measured_compute_throughput_and_deadline(self):
        result = evaluate_windows([150, 200, 220, 230, 250], max_window_ms=400)
        self.assertTrue(result["meets_compute_deadline"])
        self.assertEqual(result["samples"], 5)
        self.assertEqual(result["worst_window_ms"], 250)
        self.assertEqual(result["p95_window_ms"], 250)
        self.assertGreater(result["observed_processing_fps"], 25)

    def test_overloaded_gpu_never_passes_on_mean_alone(self):
        result = evaluate_windows([100, 100, 100, 100, 500], max_window_ms=400)
        self.assertFalse(result["meets_compute_deadline"])
        self.assertEqual(result["worst_window_ms"], 500)

    def test_invalid_samples_never_produce_positive_evidence(self):
        for durations in ([], [0], [-1, 200], [float("nan")], [float("inf")]):
            with self.subTest(durations=durations):
                with self.assertRaises(ValueError):
                    evaluate_windows(durations, max_window_ms=400)
        for deadline in (0, -1, float("nan"), float("inf"), float("-inf"), True):
            with self.subTest(deadline=deadline), self.assertRaises(ValueError):
                evaluate_windows([100], max_window_ms=deadline)
        with self.assertRaises(ValueError):
            evaluate_windows([True, 100], max_window_ms=400)


    def test_gpu_frame_contract_rejects_shape_drift_and_non_bgr_bytes(self):
        class Frame:
            def __init__(self, shape=(256, 256, 3), dtype="uint8"):
                self.shape = shape
                self.ndim = len(shape)
                self.dtype = dtype

        valid = [Frame() for _ in range(10)]
        self.assertEqual(validate_frame_batch(valid), (256, 256, 3))
        self.assertEqual(validate_frame_batch(valid, (256, 256, 3)), (256, 256, 3))
        invalid = [
            [],
            valid[:-1],
            valid[:-1] + [Frame(shape=(128, 256, 3))],
            valid[:-1] + [Frame(shape=(256, 256, 4))],
            valid[:-1] + [Frame(shape=(0, 256, 3))],
            valid[:-1] + [Frame(dtype="float32")],
            [None] + valid[1:],
        ]
        for batch in invalid:
            with self.subTest(batch=batch), self.assertRaises(RuntimeError):
                validate_frame_batch(batch)
        with self.assertRaises(RuntimeError):
            validate_frame_batch(valid, (128, 256, 3))


if __name__ == "__main__":
    unittest.main()
