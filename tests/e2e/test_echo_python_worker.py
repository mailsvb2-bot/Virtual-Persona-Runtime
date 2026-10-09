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

    async def aclose(self):
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

    def __init__(self):
        self.trace = {
            "audio": [], "closed": 0, "stream_options": [], "stops": [],
            "connected": [], "disconnected": 0,
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


def run_worker(delay_after_speak, records=None, delay_before_close=0):
    spec = importlib.util.spec_from_file_location("vpr_echo_worker", WORKER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    def synthesize(text):
        if not delay_after_speak:
            time.sleep(0.12)
        return b"RIFF" + text.encode("utf-8")

    module.synthesize = synthesize
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

    def test_immediate_revoke_discards_unspoken_generation(self):
        trace, receipts = run_worker(False)
        self.assertFalse(trace["audio"], "revoked utterance must not publish audio")
        self.assertEqual(trace["stops"],
                         [("{}", "did.interrupt"), ("{}", "did.interrupt")])
        self.assertFalse(any(item["id"] == 1 and item["ok"] for item in receipts))
        self.assertTrue(any(item["id"] == 2 and item["ok"] for item in receipts))


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


if __name__ == "__main__":
    unittest.main()
