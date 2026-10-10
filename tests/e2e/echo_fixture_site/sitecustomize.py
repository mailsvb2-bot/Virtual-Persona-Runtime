"""Hermetic LiveKit stand-in for the Expressive Echo end-to-end subprocess.
Loaded only by explicit VPR fixture PYTHONPATH, never by production workers.
It runs the actual Echo worker Python, including its STOP fencing and TTS.
"""
import asyncio
import json
import sys
import types
import urllib.request

def emit_event(kind):
    payload = json.dumps({"kind": kind}).encode("utf-8")
    req = urllib.request.Request("http://127.0.0.1:18790/__echo-event", data=payload,
                                 headers={"content-type": "application/json"}, method="POST")
    with urllib.request.urlopen(req, timeout=2) as response:
        if response.status != 200:
            raise RuntimeError("Echo fixture telemetry rejected")

class FakeWriter:
    async def write(self, chunk):
        if not chunk:
            raise ValueError("empty speech bytes")
        emit_event("audio_bytes_written")
    async def aclose(self):
        emit_event("audio_stream_closed")
        await asyncio.sleep(0)

class FakeParticipant:
    async def stream_bytes(self, *, topic, attributes, destination_identities, **_):
        if topic != "did.audio-stream" or attributes != {"format": "wav"} or not destination_identities:
            raise ValueError("invalid server-owned Echo audio publication")
        emit_event("audio_stream_opened")
        return FakeWriter()
    async def send_text(self, text, *, topic):
        if topic != "did.interrupt" or text != "{}":
            raise ValueError("invalid Echo STOP")
        emit_event("provider_stop_sent")
class FakeRoom:
    def __init__(self):
        self.callbacks = {}
        self.local_participant = FakeParticipant()
        self.remote_participants = {"agent-1": types.SimpleNamespace(identity="agent-1", kind="agent")}
    def on(self, event, callback):
        self.callbacks[event] = callback
    async def connect(self, url, token):
        if not url.startswith("wss://") or not token.startswith("fixture-private-echo-token-"):
            raise ValueError("unexpected Echo private transport")
        self.callbacks["track_subscribed"](None, None, None)
        emit_event("private_echo_connected")
    async def disconnect(self):
        emit_event("private_echo_disconnected")
rtc = types.SimpleNamespace(Room=FakeRoom, ParticipantKind=types.SimpleNamespace(PARTICIPANT_KIND_AGENT="agent"))
module = types.ModuleType("livekit")
module.rtc = rtc
sys.modules["livekit"] = module
