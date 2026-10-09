"""Private D-ID Echo LiveKit sender. Runs only as an explicitly enabled local subprocess.

No browser gets echo_token, OpenAI TTS credentials, raw audio or did.speak access.
Protocol: single JSON request per stdin line, sanitized {id, ok} receipts on stdout.
"""
import asyncio
import io
import json
import os
import sys
import urllib.request
import wave
from urllib.parse import urlsplit

MAX_WAV_BYTES = 8 * 1024 * 1024
MAX_TEXT_BYTES = 4096
TTS_TIMEOUT_SECONDS = 35


def emit(identifier, ok):
    print(json.dumps({"id": identifier, "ok": bool(ok)}, separators=(",", ":")), flush=True)


def synthesize(text):
    """Blocking TTS lives in a worker thread; cancellation fences its result."""
    if not isinstance(text, str) or not text.strip():
        raise ValueError("invalid speech")
    if len(text.encode("utf-8")) > MAX_TEXT_BYTES:
        raise ValueError("speech too long")
    payload = json.dumps({
        "model": os.environ["VPR_DID_ECHO_TTS_MODEL"],
        "voice": os.environ["VPR_DID_ECHO_TTS_VOICE"],
        "input": text,
        "response_format": "wav",
    }, ensure_ascii=False).encode("utf-8")
    request = urllib.request.Request(
        os.environ["VPR_DID_ECHO_TTS_ENDPOINT"],
        data=payload,
        headers={
            "Authorization": "Bearer " + os.environ["VPR_DID_ECHO_TTS_API_KEY"],
            "Content-Type": "application/json",
        },
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=TTS_TIMEOUT_SECONDS) as response:
        wav_bytes = response.read(MAX_WAV_BYTES + 1)
    if len(wav_bytes) > MAX_WAV_BYTES or not wav_bytes.startswith(b"RIFF"):
        raise ValueError("invalid bounded WAV")
    with wave.open(io.BytesIO(wav_bytes), "rb") as audio:
        if (
            audio.getcomptype() != "NONE"
            or audio.getsampwidth() != 2
            or audio.getnchannels() not in (1, 2)
            or audio.getframerate() < 8000
            or audio.getnframes() == 0
            or audio.getnframes() > audio.getframerate() * 60
        ):
            raise ValueError("unsupported audio format")
    return wav_bytes


async def run():
    from livekit import rtc

    raw = await asyncio.to_thread(sys.stdin.readline)
    initial = json.loads(raw)
    url = initial["session_url"]
    token = initial["echo_token"]
    if initial.get("command") != "open" or not isinstance(url, str):
        raise ValueError("invalid open")
    parts = urlsplit(url)
    if parts.scheme != "wss" or not parts.hostname or parts.username or parts.password:
        raise ValueError("invalid LiveKit origin")
    origin = url.split("/room/", 1)[0]
    room = rtc.Room()
    ready = asyncio.Event()
    room.on("track_subscribed", lambda *_: ready.set())
    try:
        await asyncio.wait_for(room.connect(origin, token), timeout=25)
        # D-ID drops audio published before its avatar media tracks are ready.
        await asyncio.wait_for(ready.wait(), timeout=25)
    except Exception:
        emit(initial.get("id", 0), False)
        await room.disconnect()
        return
    agents = [
        participant.identity
        for participant in room.remote_participants.values()
        if participant.kind == rtc.ParticipantKind.PARTICIPANT_KIND_AGENT
        or participant.identity.startswith("agent")
    ]
    if not agents:
        emit(initial.get("id", 0), False)
        await room.disconnect()
        return
    emit(initial.get("id", 0), True)

    utterance_lock = asyncio.Lock()
    tasks = {}
    generation = 0

    async def speak(identifier, phrase, generation_at_start):
        writer = None
        try:
            # Serialize synthesis as well as sending. Otherwise a later,
            # faster TTS request can overtake an earlier spoken sentence.
            async with utterance_lock:
                if generation_at_start != generation:
                    emit(identifier, False)
                    return
                wav = await asyncio.to_thread(synthesize, phrase)
                if generation_at_start != generation:
                    emit(identifier, False)
                    return
                writer = await room.local_participant.stream_bytes(
                    name=f"rt0-utterance-{identifier}",
                    topic="did.audio-stream",
                    attributes={"format": "wav"},
                    destination_identities=agents,
                )
                # Backpressure from LiveKit bounds outbound sender memory.
                for offset in range(0, len(wav), 16 * 1024):
                    if generation_at_start != generation:
                        raise asyncio.CancelledError()
                    await writer.write(wav[offset:offset + 16 * 1024])
                await writer.aclose()
                writer = None
                emit(identifier, True)
        except asyncio.CancelledError:
            emit(identifier, False)
        except Exception:
            emit(identifier, False)
        finally:
            if writer is not None:
                try:
                    await writer.aclose()
                except Exception:
                    pass
            # A stale task must never remove a newer task's ownership.
            if tasks.get(identifier) is asyncio.current_task():
                tasks.pop(identifier, None)

    try:
        while raw := await asyncio.to_thread(sys.stdin.readline):
            try:
                request = json.loads(raw)
                identifier = request["id"]
                command = request["command"]
                if command == "speak":
                    text = request["text"]
                    # Request IDs identify exactly one in-flight utterance. A duplicate
                    # must not replace its task or evade STOP/close cancellation.
                    if identifier in tasks:
                        emit(identifier, False)
                        continue
                    if (
                        not isinstance(text, str)
                        or not text.strip()
                        or len(text.encode("utf-8")) > MAX_TEXT_BYTES
                    ):
                        emit(identifier, False)
                        continue
                    task = asyncio.create_task(speak(identifier, text, generation))
                    tasks[identifier] = task
                elif command == "interrupt":
                    generation += 1
                    for task in tuple(tasks.values()):
                        task.cancel()
                    # This is a text STOP command, never browser-published audio.
                    await room.local_participant.send_text("{}", topic="did.interrupt")
                    emit(identifier, True)
                elif command == "close":
                    generation += 1
                    for task in tuple(tasks.values()):
                        task.cancel()
                    await room.local_participant.send_text("{}", topic="did.interrupt")
                    emit(identifier, True)
                    break
                else:
                    emit(identifier, False)
            except Exception:
                # No exception text, credentials or user speech in stdout.
                try:
                    emit(request.get("id", -1), False)
                except Exception:
                    pass
    finally:
        for task in tuple(tasks.values()):
            task.cancel()
        if tasks:
            await asyncio.gather(*tuple(tasks.values()), return_exceptions=True)
        await room.disconnect()


if __name__ == "__main__":
    try:
        asyncio.run(run())
    except Exception:
        # Process-level failure cannot be mistaken for a successful STOP.
        sys.exit(1)
