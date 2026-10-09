"""Hermetic LiveKit stand-in for the Expressive Echo end-to-end subprocess.
Loaded only by explicit VPR fixture PYTHONPATH, never by production workers.
It runs the actual Echo worker Python, including its STOP fencing and TTS.
"""
import asyncio
import sys
import types

class FakeWriter:
    async def write(self, chunk):
        if not chunk:
            raise ValueError("empty speech bytes")
    async def aclose(self):
        await asyncio.sleep(0)

class FakeParticipant:
    async def stream_bytes(self, *, topic, attributes, destination_identities, **_):
        if topic != "did.audio-stream" or attributes != {"format": "wav"} or not destination_identities:
            raise ValueError("invalid server-owned Echo audio publication")
        return FakeWriter()
    async def send_text(self, text, *, topic):
        if topic != "did.interrupt" or text != "{}":
            raise ValueError("invalid Echo STOP")
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
    async def disconnect(self):
        pass
rtc = types.SimpleNamespace(Room=FakeRoom, ParticipantKind=types.SimpleNamespace(PARTICIPANT_KIND_AGENT="agent"))
module = types.ModuleType("livekit")
module.rtc = rtc
sys.modules["livekit"] = module
