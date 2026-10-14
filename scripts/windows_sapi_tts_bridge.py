"""Protected, persistent Windows native Russian SAPI synthesis for Echo.

Only a bounded base64 IPC protocol carries user text; no user text on process
arguments, logs, disk, or network. One reusable PowerShell speech host avoids
starting PowerShell for EVERY clause. The voice remains Windows SAPI and is
not represented as human-quality neural speech.
"""
from __future__ import annotations

import atexit
import base64
import io
import os
import queue
import subprocess
import threading
import wave
from http.server import ThreadingHTTPServer

from metrotrance_tts_bridge import make_handler, validate_echo_wav

MAX_TEXT_BYTES = 4096
MAX_WAV_BYTES = 8 * 1024 * 1024
MAX_BASE64_LINE = ((MAX_WAV_BYTES + 2) // 3) * 4 + 2
START_TIMEOUT_SECONDS = 15
VOICE_TIMEOUT_SECONDS = 35
RATE_ENV = "VPR_WINDOWS_SAPI_RATE"

# Static code only. The text enters on stdin as a separate base64 line.
# The PowerShell process stays alive for subsequent phrases; one synth object
# and voice selection is reused, rather than creating a new OS process per turn.
_PS_SCRIPT = r"""
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Speech
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
try {
    $target = "Microsoft Irina Desktop"
    $installed = @($synth.GetInstalledVoices() | Where-Object {
        $_.Enabled -and $_.VoiceInfo.Name -eq $target -and
        $_.VoiceInfo.Culture.Name -eq "ru-RU" -and
        $_.VoiceInfo.Gender.ToString() -eq "Female"
    })
    if ($installed.Count -ne 1) { throw "Russian female Windows voice unavailable" }
    $synth.SelectVoice($target)
    $synth.Rate = [int]([Environment]::GetEnvironmentVariable("VPR_WINDOWS_SAPI_RATE"))
    [Console]::Out.WriteLine("READY")
    [Console]::Out.Flush()
    while ($null -ne ($input64 = [Console]::In.ReadLine())) {
        if ($input64 -ceq "QUIT") { break }
        try {
            $text = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($input64))
            if ([string]::IsNullOrWhiteSpace($text) -or [Text.Encoding]::UTF8.GetByteCount($text) -gt 4096) {
                throw "Invalid speech length"
            }
            $buffer = New-Object System.IO.MemoryStream
            try {
                $synth.SetOutputToWaveStream($buffer)
                $synth.Speak($text)
                [Console]::Out.WriteLine([Convert]::ToBase64String($buffer.ToArray()))
            } finally {
                $synth.SetOutputToNull()
                $buffer.Dispose()
            }
        } catch {
            # Never echo phrases, errors, or private provider configuration.
            [Console]::Out.WriteLine("ERR")
        }
        [Console]::Out.Flush()
    }
} finally {
    $synth.Dispose()
}
"""


def _rate() -> int:
    raw = os.environ.get(RATE_ENV, "1")
    try:
        rate = int(raw)
    except ValueError as error:
        raise ValueError("Windows SAPI rate must be an integer between -2 and 3") from error
    if not -2 <= rate <= 3:
        raise ValueError("Windows SAPI rate must be between -2 and 3")
    return rate


def _bounded_readline(process, timeout: float) -> str:
    """Read a single bounded line; terminate the worker if it stalls."""
    if process.stdout is None:
        raise RuntimeError("Windows SAPI output unavailable")
    responses = queue.Queue(maxsize=1)

    def read():
        try:
            responses.put((process.stdout.readline(MAX_BASE64_LINE), None))
        except (OSError, UnicodeError, ValueError) as error:
            responses.put((None, error))

    reader = threading.Thread(target=read, name="vpr-sapi-response", daemon=True)
    reader.start()
    try:
        line, error = responses.get(timeout=timeout)
    except queue.Empty as error:
        raise TimeoutError("Windows SAPI process exceeded speech timeout") from error
    if error or not line or len(line) >= MAX_BASE64_LINE or not line.endswith("\n"):
        raise RuntimeError("Windows SAPI protocol response invalid")
    return line.strip()


class WindowsSapiSynthesizer:
    """One private child process per bridge, serialized by a bounded lock."""

    def __init__(self, speech_rate: int, *, startup_timeout=START_TIMEOUT_SECONDS,
                 voice_timeout=VOICE_TIMEOUT_SECONDS):
        if not -2 <= speech_rate <= 3:
            raise ValueError("invalid native voice rate")
        self._rate = speech_rate
        self._startup_timeout = startup_timeout
        self._voice_timeout = voice_timeout
        self._lock = threading.Lock()
        self._process = None

    def _discard(self):
        child = self._process
        self._process = None
        if child is None:
            return
        if child.poll() is None:
            child.kill()
        try:
            child.wait(timeout=3)
        except subprocess.TimeoutExpired:
            pass
        for stream in (child.stdin, child.stdout):
            if stream:
                try:
                    stream.close()
                except OSError:
                    pass

    def _ensure_started(self):
        if self._process is not None and self._process.poll() is None:
            return self._process
        self._discard()
        # Positive allowlist: child speech host needs OS/voice paths, not
        # application, payment, LLM, STT, or avatar credentials of any name.
        allowed = {
            "SYSTEMROOT", "WINDIR", "PATH", "PATHEXT", "TEMP", "TMP",
            "USERPROFILE", "APPDATA", "LOCALAPPDATA", "PSMODULEPATH",
            "COMSPEC", "SYSTEMDRIVE", "HOMEDRIVE", "HOMEPATH",
        }
        safe_env = {key: val for key, val in os.environ.items()
                    if key.upper() in allowed}
        safe_env[RATE_ENV] = str(self._rate)
        command = [
            "powershell.exe", "-NoProfile", "-NonInteractive", "-EncodedCommand",
            base64.b64encode(_PS_SCRIPT.encode("utf-16-le")).decode("ascii"),
        ]
        self._process = subprocess.Popen(
            command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, text=True, encoding="ascii", bufsize=1,
            env=safe_env,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        if _bounded_readline(self._process, self._startup_timeout) != "READY":
            raise RuntimeError("Native voice host readiness rejected")
        return self._process

    def __call__(self, text: str) -> bytes:
        if (not isinstance(text, str) or not text.strip()
                or len(text.encode("utf-8")) > MAX_TEXT_BYTES):
            raise ValueError("invalid speech input")
        encoded = base64.b64encode(text.encode("utf-8")).decode("ascii")
        with self._lock:
            try:
                child = self._ensure_started()
                if child.stdin is None:
                    raise RuntimeError("Windows SAPI input unavailable")
                child.stdin.write(encoded + "\n")
                child.stdin.flush()
                response = _bounded_readline(child, self._voice_timeout)
                if response == "ERR":
                    raise RuntimeError("Windows Russian voice synthesis failed")
                try:
                    wav = base64.b64decode(response, validate=True)
                except (ValueError, base64.binascii.Error) as error:
                    raise RuntimeError("Invalid Windows SAPI WAV encoding") from error
                if not wav or len(wav) > MAX_WAV_BYTES or not validate_echo_wav(wav):
                    raise RuntimeError("Windows SAPI returned Echo-incompatible WAV")
                with wave.open(io.BytesIO(wav), "rb") as audio:
                    if audio.getnchannels() != 1 or audio.getsampwidth() != 2:
                        raise RuntimeError("Windows SAPI must return mono PCM16")
                return wav
            except (OSError, RuntimeError, TimeoutError, UnicodeError) as error:
                self._discard()
                raise RuntimeError("Protected Windows speech synthesis unavailable") from error

    def close(self):
        with self._lock:
            self._discard()


def make_windows_sapi_synthesizer():
    if os.name != "nt":
        raise RuntimeError("Windows system voice is available only on Windows")
    synth = WindowsSapiSynthesizer(_rate())
    atexit.register(synth.close)
    return synth


def main():
    synth = make_windows_sapi_synthesizer()
    token = os.environ.get("VPR_METROTRANCE_TTS_TOKEN", "")
    port = int(os.environ.get("VPR_METROTRANCE_TTS_PORT", "8897"))
    server = ThreadingHTTPServer(
        ("127.0.0.1", port), make_handler(token, synth, provider_id="windows-sapi"),
    )
    try:
        print("Windows Russian female TTS ready on loopback", flush=True)
        server.serve_forever()
    finally:
        server.server_close()
        synth.close()


if __name__ == "__main__":
    main()
