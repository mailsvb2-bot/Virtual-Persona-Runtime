"""Hermetic contract test for the *real* Echo worker control flow.

Only LiveKit and the paid TTS function are fake. The actual Python worker,
stream_bytes topic/attributes, generation cancellation, and STOP loop execute.
"""
import asyncio
import importlib.util
import io
import json
import sys
import time
import types
import unittest
from unittest import mock
from pathlib import Path

WORKER = (
    Path(__file__).resolve().parents[2]
    / "crates/vpr-provider-did-agent-streams/src/echo_python_worker.py"
)


class FakeWriter:
    def __init__(self, trace):
        self.trace = trace

    async def write(self, data):
        self.trace["audio"].append(bytes(data))
        self.trace["write_timestamps"].append(time.monotonic())

    async def aclose(self):
        if self.trace["close_latency"]:
            await asyncio.sleep(self.trace["close_latency"])
        self.trace["closed"] += 1


class FakeLocalParticipant:
    def __init__(self, trace):
        self.trace = trace

    async def stream_bytes(self, **options):
        self.trace["stream_options"].append(options)
        return FakeWriter(self.trace)

    async def send_text(self, text, *, topic):
        self.trace["stops"].append((text, topic))


class FakeRoom:
    instances = []
    close_latency = 0

    def __init__(self):
        self.trace = {
            "audio": [], "write_timestamps": [], "closed": 0, "stream_options": [], "stops": [],
            "connected": [], "disconnected": 0,
            "close_latency": self.__class__.close_latency,
        }
        self.local_participant = FakeLocalParticipant(self.trace)
        self.remote_participants = {
            "agent-1": types.SimpleNamespace(identity="agent-1", kind="agent")
        }
        self.callbacks = {}
        self.__class__.instances.append(self)

    def on(self, name, callback):
        self.callbacks[name] = callback

    async def connect(self, url, token):
        self.trace["connected"].append((url, token))
        self.callbacks["track_subscribed"](None, None, None)

    async def disconnect(self):
        self.trace["disconnected"] += 1


class DelayedInput:
    def __init__(self, records, delay_after_speak, delay_before_close=0):
        self.records = list(records)
        self.delay_after_speak = delay_after_speak
        self.delay_before_close = delay_before_close

    def readline(self):
        if not self.records:
            return ""
        line = self.records.pop(0)
        if self.delay_after_speak and '"command": "interrupt"' in line:
            time.sleep(0.12)
        if self.delay_before_close and '"command": "close"' in line:
            time.sleep(self.delay_before_close)
        return line


def run_worker(delay_after_speak, records=None, delay_before_close=0, tts_latency=None,
               close_latency=0, tts_chunks=None):
    spec = importlib.util.spec_from_file_location("vpr_echo_worker", WORKER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    def synthesize_chunks(text):
        if tts_latency is not None:
            time.sleep(tts_latency(text))
        elif not delay_after_speak:
            time.sleep(0.12)
        if tts_chunks is not None:
            yield from tts_chunks(text)
        else:
            yield b"RIFF" + text.encode("utf-8")

    module.synthesize_chunks = synthesize_chunks
    mock_livekit = types.ModuleType("livekit")
    mock_livekit.rtc = types.SimpleNamespace(
        Room=FakeRoom,
        ParticipantKind=types.SimpleNamespace(PARTICIPANT_KIND_AGENT="agent"),
    )
    previous = sys.modules.get("livekit")
    sys.modules["livekit"] = mock_livekit
    old_input, old_output = sys.stdin, sys.stdout
    response = io.StringIO()
    default_records = [
        {"id": 0, "command": "open",
         "session_url": "wss://livekit.example.test/room/agent-1",
         "echo_token": "private-echo-token"},
        {"id": 1, "command": "speak", "text": "Привет!"},
        {"id": 2, "command": "interrupt"},
        {"id": 3, "command": "close"},
    ]
    FakeRoom.instances.clear()
    FakeRoom.close_latency = close_latency
    try:
        sys.stdin = DelayedInput(
            [json.dumps(record, ensure_ascii=False) + "\n"
             for record in (records if records is not None else default_records)],
            delay_after_speak, delay_before_close,
        )
        sys.stdout = response
        asyncio.run(module.run())
    finally:
        sys.stdin, sys.stdout = old_input, old_output
        if previous is None:
            del sys.modules["livekit"]
        else:
            sys.modules["livekit"] = previous
    return FakeRoom.instances[-1].trace, [
        json.loads(line) for line in response.getvalue().splitlines()
    ]


class EchoWorkerTests(unittest.TestCase):
    def test_completed_utterance_is_private_byte_stream_and_stop_is_control_only(self):
        trace, receipts = run_worker(True)
        self.assertEqual(trace["connected"],
                         [("wss://livekit.example.test", "private-echo-token")])
        self.assertTrue(trace["audio"])
        self.assertEqual(trace["stream_options"][0]["topic"], "did.audio-stream")
        self.assertEqual(trace["stream_options"][0]["attributes"], {"format": "wav"})
        self.assertEqual(trace["stream_options"][0]["destination_identities"], ["agent-1"])
        self.assertTrue(all(text == "{}" and topic == "did.interrupt"
                            for text, topic in trace["stops"]))
        self.assertEqual(trace["disconnected"], 1)
        self.assertTrue(any(item["id"] == 1 and item["ok"] for item in receipts))
        self.assertTrue(any(item["id"] == 2 and item["ok"] for item in receipts))

    def test_canonical_pcm_is_published_as_private_wav_without_worker_tts(self):
        pcm = bytes(320)
        records = [
            {"id": 0, "command": "open",
             "session_url": "wss://livekit.example.test/room/agent-1",
             "echo_token": "private-echo-token"},
            {"id": 1, "command": "pcm", "pcm_hex": pcm.hex(),
             "sample_rate_hz": 16000, "channels": 1},
            {"id": 2, "command": "close"},
        ]
        trace, receipts = run_worker(True, records=records, delay_before_close=0.15)
        self.assertTrue(any(item["id"] == 1 and item["ok"] for item in receipts))
        audio = b"".join(trace["audio"])
        self.assertTrue(audio.startswith(b"RIFF"))
        self.assertEqual(trace["stream_options"][0]["topic"], "did.audio-stream")
        with __import__("wave").open(io.BytesIO(audio), "rb") as wav:
            self.assertEqual(wav.getframerate(), 16000)
            self.assertEqual(wav.getnchannels(), 1)
            self.assertEqual(wav.readframes(160), pcm)

    def test_invalid_pcm_never_reaches_private_publisher(self):
        records = [
            {"id": 0, "command": "open",
             "session_url": "wss://livekit.example.test/room/agent-1",
             "echo_token": "private-echo-token"},
            {"id": 1, "command": "pcm", "pcm_hex": "00",
             "sample_rate_hz": 16000, "channels": 1},
            {"id": 2, "command": "close"},
        ]
        trace, receipts = run_worker(True, records=records)
        self.assertFalse(trace["audio"])
        self.assertTrue(any(item["id"] == 1 and not item["ok"] for item in receipts))

    def test_immediate_revoke_discards_unspoken_generation(self):
        trace, receipts = run_worker(False)
        self.assertFalse(trace["audio"], "revoked utterance must not publish audio")
        self.assertEqual(trace["stops"],
                         [("{}", "did.interrupt"), ("{}", "did.interrupt")])
        self.assertFalse(any(item["id"] == 1 and item["ok"] for item in receipts))
        self.assertTrue(any(item["id"] == 2 and item["ok"] for item in receipts))


    def test_out_of_order_tts_cannot_reorder_canonical_speech(self):
        records = [
            {"id": 0, "command": "open",
             "session_url": "wss://livekit.example.test/room/agent-1",
             "echo_token": "private-echo-token"},
            {"id": 7, "command": "speak", "text": "Первое предложение."},
            {"id": 8, "command": "speak", "text": "Второе предложение."},
            {"id": 9, "command": "close"},
        ]
        trace, receipts = run_worker(
            False, records=records, delay_before_close=0.30,
            tts_latency=lambda text: 0.12 if text.startswith("Первое") else 0.01,
        )
        audio = b"".join(trace["audio"])
        self.assertEqual(len(trace["stream_options"]), 2)
        self.assertLess(audio.index("Первое".encode()), audio.index("Второе".encode()))
        self.assertTrue(all(any(r["id"] == idx and r["ok"] for r in receipts)
                            for idx in (7, 8)))

    def test_interrupt_drops_queued_second_phrase_before_audio_publication(self):
        records = [
            {"id": 0, "command": "open",
             "session_url": "wss://livekit.example.test/room/agent-1",
             "echo_token": "private-echo-token"},
            {"id": 10, "command": "speak", "text": "Первая фраза"},
            {"id": 11, "command": "speak", "text": "Вторая фраза"},
            {"id": 12, "command": "interrupt"},
            {"id": 13, "command": "close"},
        ]
        trace, receipts = run_worker(
            True, records=records,
            tts_latency=lambda text: 0.01 if text.startswith("Первая") else 0.3,
        )
        audio = b"".join(trace["audio"])
        self.assertIn("Первая фраза".encode("utf-8"), audio)
        self.assertNotIn("Вторая фраза".encode("utf-8"), audio)
        self.assertTrue(any(r["id"] == 12 and r["ok"] for r in receipts))
        self.assertFalse(any(r["id"] == 11 and r["ok"] for r in receipts))

    def test_interrupt_during_livekit_stream_close_is_not_acked_as_success(self):
        records = [
            {"id": 0, "command": "open",
             "session_url": "wss://livekit.example.test/room/agent-1",
             "echo_token": "private-echo-token"},
            {"id": 21, "command": "speak", "text": "Завершение речи"},
            {"id": 22, "command": "interrupt"},
            {"id": 23, "command": "close"},
        ]
        trace, receipts = run_worker(
            True, records=records, tts_latency=lambda _: 0,
            close_latency=0.30,
        )
        self.assertTrue(trace["audio"])
        self.assertFalse(any(r["id"] == 21 and r["ok"] for r in receipts))
        self.assertTrue(any(r["id"] == 22 and r["ok"] for r in receipts))

    def test_echo_publishes_first_chunk_before_tts_response_is_complete(self):
        records = [
            {"id": 0, "command": "open",
             "session_url": "wss://livekit.example.test/room/agent-1",
             "echo_token": "private-echo-token"},
            {"id": 31, "command": "speak", "text": "Привет"},
            {"id": 32, "command": "close"},
        ]

        def slow_chunks(_):
            yield b"RIFF-first-wav-chunk"
            time.sleep(0.15)
            yield b"second-wav-chunk"

        trace, receipts = run_worker(
            True, records=records, delay_before_close=0.4,
            tts_chunks=slow_chunks,
        )
        self.assertEqual(trace["audio"], [b"RIFF-first-wav-chunk", b"second-wav-chunk"])
        self.assertGreaterEqual(
            trace["write_timestamps"][1] - trace["write_timestamps"][0],
            0.10,
            "private LiveKit sender must receive first chunk BEFORE TTS EOF",
        )
        self.assertTrue(any(item["id"] == 31 and item["ok"] for item in receipts))

    def test_stop_during_stream_fences_late_tts_chunks(self):
        records = [
            {"id": 0, "command": "open",
             "session_url": "wss://livekit.example.test/room/agent-1",
             "echo_token": "private-echo-token"},
            {"id": 41, "command": "speak", "text": "Привет"},
            {"id": 42, "command": "interrupt"},
            {"id": 43, "command": "close"},
        ]

        def slow_chunks(_):
            yield b"RIFF-prefix"
            time.sleep(0.3)
            yield b"FORBIDDEN-LATE-AUDIO"

        trace, receipts = run_worker(
            True, records=records, delay_before_close=0.05,
            tts_chunks=slow_chunks,
        )
        self.assertIn(b"RIFF-prefix", trace["audio"])
        self.assertNotIn(b"FORBIDDEN-LATE-AUDIO", trace["audio"])
        self.assertTrue(any(item["id"] == 42 and item["ok"] for item in receipts))
        self.assertFalse(any(item["id"] == 41 and item["ok"] for item in receipts))

    def test_failed_stream_interrupts_partially_sent_wav_and_fences_queue(self):
        records = [
            {"id": 0, "command": "open",
             "session_url": "wss://livekit.example.test/room/agent-1",
             "echo_token": "private-echo-token"},
            {"id": 51, "command": "speak", "text": "Первая фраза"},
            {"id": 52, "command": "speak", "text": "Вторая фраза"},
            {"id": 53, "command": "close"},
        ]

        def chunks(text):
            if text.startswith("Первая"):
                yield b"RIFF-incomplete"
                raise ValueError("provider returned a truncated WAV")
            yield b"FORBIDDEN-QUEUED-AUDIO"

        trace, receipts = run_worker(
            True, records=records, delay_before_close=0.3, tts_chunks=chunks,
        )
        self.assertIn(b"RIFF-incomplete", trace["audio"])
        self.assertNotIn(b"FORBIDDEN-QUEUED-AUDIO", trace["audio"])
        self.assertTrue(any(topic == "did.interrupt" for _, topic in trace["stops"]))
        self.assertFalse(any(item["id"] == 51 and item["ok"] for item in receipts))
        self.assertFalse(any(item["id"] == 52 and item["ok"] for item in receipts))

    def test_queued_echo_utterances_have_a_hard_limit(self):
        records = [
            {"id": 0, "command": "open",
             "session_url": "wss://livekit.example.test/room/agent-1",
             "echo_token": "private-echo-token"},
            *({"id": idx, "command": "speak", "text": "Фраза"}
              for idx in range(1, 18)),
            {"id": 20, "command": "close"},
        ]
        trace, receipts = run_worker(
            False, records=records, delay_before_close=0.05,
            tts_latency=lambda _: 0.3,
        )
        self.assertFalse(any(r["id"] == 17 and r["ok"] for r in receipts))
        self.assertTrue(any(r["id"] == 17 and not r["ok"] for r in receipts))
        self.assertFalse(trace["audio"])
        self.assertEqual(trace["disconnected"], 1)

    def test_duplicate_utterance_id_does_not_replace_active_task(self):
        records = [
            {"id": 0, "command": "open",
             "session_url": "wss://livekit.example.test/room/agent-1",
             "echo_token": "private-echo-token"},
            {"id": 7, "command": "speak", "text": "Первый ответ"},
            {"id": 7, "command": "speak", "text": "Второй ответ"},
            {"id": 8, "command": "close"},
        ]
        # Hold the close input until the first TTS has completed. Both speech
        # commands arrive without awaiting synthesis; the duplicate must fail.
        trace, receipts = run_worker(
            False, records=records, delay_before_close=0.25,
        )
        self.assertEqual(len(trace["stream_options"]), 1)
        self.assertIn("Первый ответ".encode("utf-8"), b"".join(trace["audio"]))
        self.assertNotIn("Второй ответ".encode("utf-8"), b"".join(trace["audio"]))
        self.assertEqual(
            sorted(item["ok"] for item in receipts if item["id"] == 7),
            [False, True],
        )



class FragmentedWavResponse(io.BytesIO):
    """An HTTP-like response yielding arbitrary short PCM frames."""
    headers = {}

    def read(self, count=-1):
        return super().read(min(count, 37))


def load_worker_module():
    spec = importlib.util.spec_from_file_location("vpr_echo_stream_under_test", WORKER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def real_wav():
    import wave
    with io.BytesIO() as output:
        with wave.open(output, "wb") as audio:
            audio.setnchannels(1)
            audio.setsampwidth(2)
            audio.setframerate(24000)
            audio.writeframes(bytes((1, 0)) * 2400)
        return output.getvalue()


class EchoStreamingHttpTests(unittest.TestCase):
    def read_stream(self, wav):
        module = load_worker_module()
        response = FragmentedWavResponse(wav)
        opener = types.SimpleNamespace(open=lambda *args, **kwargs: response)
        private_environment = {
            "VPR_DID_ECHO_TTS_MODEL": "test-model",
            "VPR_DID_ECHO_TTS_VOICE": "test-voice",
            "VPR_DID_ECHO_TTS_ENDPOINT": "https://speech.example.test/v1/audio/speech",
            "VPR_DID_ECHO_TTS_API_KEY": "test-key",
        }
        with mock.patch.dict(module.os.environ, private_environment):
            with mock.patch.object(module.urllib.request, "build_opener", return_value=opener):
                parts = []
                for part in module.synthesize_chunks("Здравствуйте!"):
                    parts.append(part)
        return parts

    def test_real_wav_http_chunks_arrive_without_full_body_buffering(self):
        wav = real_wav()
        parts = self.read_stream(wav)
        self.assertGreater(len(parts), 1, "must yield WAV header before the full file")
        self.assertLess(len(parts[0]), len(wav), "first chunk cannot be the whole response")
        self.assertEqual(b"".join(parts), wav)

    def test_truncated_wav_stream_fails_after_a_partial_prefix(self):
        module = load_worker_module()
        wav = real_wav()
        response = FragmentedWavResponse(wav[:-1])
        opener = types.SimpleNamespace(open=lambda *args, **kwargs: response)
        private_environment = {
            "VPR_DID_ECHO_TTS_MODEL": "test-model",
            "VPR_DID_ECHO_TTS_VOICE": "test-voice",
            "VPR_DID_ECHO_TTS_ENDPOINT": "https://speech.example.test/v1/audio/speech",
            "VPR_DID_ECHO_TTS_API_KEY": "test-key",
        }
        with mock.patch.dict(module.os.environ, private_environment):
            with mock.patch.object(module.urllib.request, "build_opener", return_value=opener):
                stream = module.synthesize_chunks("Привет!")
                self.assertTrue(next(stream).startswith(b"RIFF"))
                with self.assertRaisesRegex(ValueError, "incomplete streamed WAV"):
                    list(stream)

    def test_redirect_cannot_forward_private_token_or_speech(self):
        module = load_worker_module()
        with self.assertRaises(ValueError):
            module.NoCredentialRedirect().redirect_request(
                None, None, 307, "Redirect", {},
                "https://untrusted.example.test/steal",
            )


if __name__ == "__main__":
    unittest.main()
