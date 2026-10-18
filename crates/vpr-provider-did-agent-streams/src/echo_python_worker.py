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
MAX_WAV_HEADER_BYTES = 64 * 1024
WAV_STREAM_CHUNK_BYTES = 4 * 1024
MAX_TEXT_BYTES = 4096
MAX_PENDING_UTTERANCES = 16
MAX_STREAMED_PCM_QUEUE = 8
PCM_QUEUE_BACKPRESSURE_SECONDS = 0.15
MAX_PCM_COMMAND_HEX = 32 * 1024
TTS_TIMEOUT_SECONDS = 35


def emit(identifier, ok):
    print(json.dumps({"id": identifier, "ok": bool(ok)}, separators=(",", ":")), flush=True)


class NoCredentialRedirect(urllib.request.HTTPRedirectHandler):
    """Never forward a private TTS token or voice text to a redirect target."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError("TTS redirects are not authorized")


def synthesize_chunks(text):
    """Read validated PCM16 WAV as bounded provider chunks, not a whole file.

    Called only by the private authorized Echo worker. The caller's generation
    fence is checked before EVERY async LiveKit write, including after STOP.
    """
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
    opener = urllib.request.build_opener(NoCredentialRedirect())
    with opener.open(request, timeout=TTS_TIMEOUT_SECONDS) as response:
        declared = response.headers.get("Content-Length")
        if declared is not None and int(declared) > MAX_WAV_BYTES:
            raise ValueError("oversized declared WAV")
        received = 0
        expected_size = None
        header = bytearray()
        while True:
            chunk = response.read(WAV_STREAM_CHUNK_BYTES)
            if not chunk:
                break
            received += len(chunk)
            if received > MAX_WAV_BYTES:
                raise ValueError("oversized streamed WAV")
            if expected_size is None:
                header.extend(chunk)
                if len(header) < 12:
                    continue
                if header[:4] != b"RIFF" or header[8:12] != b"WAVE":
                    raise ValueError("not PCM WAV")
                expected = int.from_bytes(header[4:8], "little") + 8
                if expected > MAX_WAV_BYTES or expected < 44 or received > expected:
                    raise ValueError("invalid declared RIFF length")
                try:
                    with wave.open(io.BytesIO(header), "rb") as audio:
                        if (
                            audio.getcomptype() != "NONE"
                            or audio.getsampwidth() != 2
                            or audio.getnchannels() not in (1, 2)
                            or not 8000 <= audio.getframerate() <= 96000
                            or audio.getnframes() == 0
                            or audio.getnframes() > audio.getframerate() * 60
                        ):
                            raise ValueError("unsupported audio format")
                except (EOFError, wave.Error) as error:
                    if len(header) >= MAX_WAV_HEADER_BYTES:
                        raise ValueError("invalid streamed WAV header") from error
                    continue
                expected_size = expected
                yield bytes(header)
                header.clear()
            else:
                if received > expected_size:
                    raise ValueError("extra untrusted WAV bytes")
                yield chunk
        if expected_size is None or received != expected_size:
            raise ValueError("incomplete streamed WAV")


def synthesize(text):
    """Compatibility buffered helper; realtime Echo uses synthesize_chunks."""
    return b"".join(synthesize_chunks(text))


def next_speech_chunk(iterator):
    """Avoid leaking StopIteration into asyncio.to_thread's Future."""
    try:
        return next(iterator)
    except StopIteration:
        return None


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
    active_pcm = None

    async def pump_pcm(state):
        nonlocal generation, active_pcm
        writer = None
        published = False
        try:
            async with utterance_lock:
                if state["generation"] != generation:
                    raise asyncio.CancelledError()
                writer = await room.local_participant.stream_bytes(
                    name=f'rt0-pcm-stream-{state["stream_id"]}',
                    topic="did.audio-stream",
                    attributes={
                        "format": "pcm16",
                        "sample_rate": str(state["rate"]),
                        "channels": str(state["channels"]),
                    },
                    destination_identities=agents,
                )
                while True:
                    chunk = await state["queue"].get()
                    if state["generation"] != generation:
                        raise asyncio.CancelledError()
                    if chunk is None:
                        break
                    await writer.write(chunk)
                    published = True
                await writer.aclose()
                writer = None
                if state["generation"] != generation:
                    raise asyncio.CancelledError()
                if state["end_id"] is not None:
                    emit(state["end_id"], True)
                    state["end_id"] = None
        except asyncio.CancelledError:
            if state["end_id"] is not None:
                emit(state["end_id"], False)
                state["end_id"] = None
        except Exception:
            if state["generation"] == generation:
                generation += 1
                if published:
                    try:
                        await room.local_participant.send_text("{}", topic="did.interrupt")
                    except Exception:
                        pass
            if state["end_id"] is not None:
                emit(state["end_id"], False)
                state["end_id"] = None
        finally:
            if writer is not None:
                try:
                    await writer.aclose()
                except Exception:
                    pass
            if active_pcm is state:
                active_pcm = None

    async def speak(identifier, phrase, generation_at_start, preauthorized_pcm=None):
        nonlocal generation
        writer = None
        published = False
        try:
            # Serialize synthesis and publishing: later TTS cannot overtake the
            # earlier sentence. Each blocking read occupies a worker thread,
            # not the STOP/control event loop.
            async with utterance_lock:
                if generation_at_start != generation:
                    emit(identifier, False)
                    return
                if preauthorized_pcm is None:
                    source = synthesize_chunks(phrase)
                    attributes = {"format": "wav"}
                    async def next_chunk():
                        return await asyncio.to_thread(next_speech_chunk, source)
                else:
                    pcm, rate, channels = preauthorized_pcm
                    attributes = {
                        "format": "pcm16", "sample_rate": str(rate),
                        "channels": str(channels),
                    }
                    chunks = iter(pcm[offset:offset + 16 * 1024]
                                  for offset in range(0, len(pcm), 16 * 1024))
                    async def next_chunk():
                        return next(chunks, None)

                chunk = await next_chunk()
                if generation_at_start != generation:
                    raise asyncio.CancelledError()
                if not chunk:
                    raise ValueError("empty TTS stream")
                writer = await room.local_participant.stream_bytes(
                    name=f"rt0-utterance-{identifier}",
                    topic="did.audio-stream",
                    attributes=attributes,
                    destination_identities=agents,
                )
                while chunk is not None:
                    if generation_at_start != generation:
                        raise asyncio.CancelledError()
                    await writer.write(chunk)
                    published = True
                    chunk = await next_chunk()
                if generation_at_start != generation:
                    raise asyncio.CancelledError()
                await writer.aclose()
                writer = None
                # A closed byte stream is only provider delivery, never verified
                # RTP playout, audio hearing, or lip synchronisation.
                if generation_at_start != generation:
                    raise asyncio.CancelledError()
                emit(identifier, True)
        except asyncio.CancelledError:
            emit(identifier, False)
        except Exception:
            # A failed stream is terminal for this publisher generation.
            # An incomplete WAV may already be in D-ID's playback queue.
            # Fence every queued utterance before sending the remote interrupt;
            # otherwise the next task can race ahead and publish stale sound.
            if generation_at_start == generation:
                generation += 1
                for task in tuple(tasks.values()):
                    if task is not asyncio.current_task():
                        task.cancel()
                if published:
                    try:
                        await room.local_participant.send_text("{}", topic="did.interrupt")
                    except Exception:
                        pass
            emit(identifier, False)
        finally:
            if writer is not None:
                try:
                    await writer.aclose()
                except Exception:
                    pass
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
                    if active_pcm is not None or identifier in tasks or len(tasks) >= MAX_PENDING_UTTERANCES:
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
                elif command == "pcm":
                    # Audio bytes originate from the canonical TTS permit in Rust.
                    # No model/key/text is accepted on this transport.
                    raw_pcm = request.get("pcm_hex")
                    rate = request.get("sample_rate_hz")
                    channels = request.get("channels")
                    if (
                        active_pcm is not None
                        or identifier in tasks
                        or len(tasks) >= MAX_PENDING_UTTERANCES
                        or not isinstance(raw_pcm, str)
                        or len(raw_pcm) > MAX_WAV_BYTES * 2
                        or not isinstance(rate, int)
                        or not isinstance(channels, int)
                        or not 8000 <= rate <= 96000
                        or channels not in (1, 2)
                    ):
                        emit(identifier, False)
                        continue
                    try:
                        pcm = bytes.fromhex(raw_pcm)
                        if not pcm or len(pcm) % (channels * 2) != 0:
                            raise ValueError("invalid PCM16")
                        if len(pcm) // (channels * 2) > rate * 60:
                            raise ValueError("audio too long")
                    except (ValueError, OverflowError):
                        emit(identifier, False)
                        continue
                    task = asyncio.create_task(speak(identifier, None, generation, (pcm, rate, channels)))
                    tasks[identifier] = task
                elif command == "pcm_begin":
                    rate = request.get("sample_rate_hz")
                    channels = request.get("channels")
                    if (
                        active_pcm is not None or tasks
                        or type(rate) is not int or type(channels) is not int
                        or not 8000 <= rate <= 96000 or channels not in (1, 2)
                    ):
                        emit(identifier, False)
                        continue
                    active_pcm = {
                        "stream_id": identifier,
                        "generation": generation,
                        "rate": rate,
                        "channels": channels,
                        "bytes": 0,
                        "end_id": None,
                        "queue": asyncio.Queue(maxsize=MAX_STREAMED_PCM_QUEUE),
                    }
                    active_pcm["task"] = asyncio.create_task(pump_pcm(active_pcm))
                    emit(identifier, True)
                elif command == "pcm_chunk":
                    state = active_pcm
                    raw_pcm = request.get("pcm_hex")
                    if (
                        state is None or state["stream_id"] != request.get("stream_id")
                        or state["generation"] != generation or state["end_id"] is not None
                        or not isinstance(raw_pcm, str) or not raw_pcm
                        or len(raw_pcm) > MAX_PCM_COMMAND_HEX
                    ):
                        emit(identifier, False)
                        continue
                    try:
                        pcm = bytes.fromhex(raw_pcm)
                        size = state["bytes"] + len(pcm)
                        if (
                            not pcm or len(pcm) % (state["channels"] * 2) != 0
                            or size > MAX_WAV_BYTES - 44
                            or size // (state["channels"] * 2) > state["rate"] * 60
                        ):
                            raise ValueError("invalid bounded PCM chunk")
                        await asyncio.wait_for(state["queue"].put(pcm), timeout=PCM_QUEUE_BACKPRESSURE_SECONDS)
                        state["bytes"] = size
                        emit(identifier, True)  # queued only, not played
                    except (ValueError, OverflowError, asyncio.TimeoutError):
                        emit(identifier, False)
                elif command == "pcm_end":
                    state = active_pcm
                    if (
                        state is None or state["stream_id"] != request.get("stream_id")
                        or state["generation"] != generation or state["end_id"] is not None
                        or state["bytes"] == 0
                    ):
                        emit(identifier, False)
                        continue
                    try:
                        await asyncio.wait_for(
                            state["queue"].put(None),
                            timeout=PCM_QUEUE_BACKPRESSURE_SECONDS,
                        )
                    except asyncio.TimeoutError:
                        emit(identifier, False)
                        continue
                    state["end_id"] = identifier
                    # The pump acknowledges end ONLY after LiveKit stream close.
                elif command == "pcm_abort":
                    state = active_pcm
                    if state is None or state["stream_id"] != request.get("stream_id"):
                        emit(identifier, False)
                        continue
                    generation += 1
                    active_pcm = None
                    state["task"].cancel()
                    if state["end_id"] is not None:
                        emit(state["end_id"], False)
                        state["end_id"] = None
                    try:
                        await room.local_participant.send_text("{}", topic="did.interrupt")
                        emit(identifier, True)
                    except Exception:
                        emit(identifier, False)
                elif command == "interrupt":
                    generation += 1
                    if active_pcm is not None:
                        state = active_pcm
                        active_pcm = None
                        state["task"].cancel()
                        if state["end_id"] is not None:
                            emit(state["end_id"], False)
                            state["end_id"] = None
                    for task in tuple(tasks.values()):
                        task.cancel()
                    # This is a text STOP command, never browser-published audio.
                    await room.local_participant.send_text("{}", topic="did.interrupt")
                    emit(identifier, True)
                elif command == "close":
                    generation += 1
                    if active_pcm is not None:
                        state = active_pcm
                        active_pcm = None
                        state["task"].cancel()
                        if state["end_id"] is not None:
                            emit(state["end_id"], False)
                            state["end_id"] = None
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
        if active_pcm is not None:
            active_pcm["task"].cancel()
            await asyncio.gather(active_pcm["task"], return_exceptions=True)
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
