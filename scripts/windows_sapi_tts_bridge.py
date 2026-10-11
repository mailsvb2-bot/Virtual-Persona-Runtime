"""Windows built-in Russian female voice via the existing protected Echo speech wire.

No network TTS, downloaded voice, reference audio or browser control. This
adapter is a source of WAV bytes only; Rust TtsPort/Policy remains the owner.
"""
from __future__ import annotations

import base64
from http.server import ThreadingHTTPServer
import io
import os
import subprocess
import threading
import wave

from metrotrance_tts_bridge import make_handler, validate_echo_wav

# Fixed code goes in the process command line; speech text is sent through
# standard input as UTF-8 base64 to preserve Cyrillic on Windows PowerShell 5.1.
_PS_SCRIPT = r"""
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Speech
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
$buffer = New-Object System.IO.MemoryStream
try {
    $target = "Microsoft Irina Desktop"
    $installed = @($synth.GetInstalledVoices() | Where-Object {
        $_.Enabled -and $_.VoiceInfo.Name -eq $target -and
        $_.VoiceInfo.Culture.Name -eq "ru-RU" -and
        $_.VoiceInfo.Gender.ToString() -eq "Female"
    })
    if ($installed.Count -ne 1) { throw "Russian female Windows voice unavailable" }
    $input64 = [Console]::In.ReadToEnd().Trim()
    $text = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($input64))
    if ([string]::IsNullOrWhiteSpace($text) -or $text.Length -gt 4096) {
        throw "Invalid speech length"
    }
    $synth.SelectVoice($target)
    $synth.SetOutputToWaveStream($buffer)
    $synth.Speak($text)
    [Console]::Out.Write([Convert]::ToBase64String($buffer.ToArray()))
} finally {
    $synth.Dispose()
    $buffer.Dispose()
}
"""


def make_windows_sapi_synthesizer():
    if os.name != "nt":
        raise RuntimeError("Windows system voice is available only on Windows")
    command = [
        "powershell.exe", "-NoProfile", "-NonInteractive", "-EncodedCommand",
        base64.b64encode(_PS_SCRIPT.encode("utf-16-le")).decode("ascii"),
    ]
    lock = threading.Lock()

    def synthesize(text: str) -> bytes:
        if not isinstance(text, str) or not text.strip() or len(text.encode("utf-8")) > 4096:
            raise ValueError("invalid speech input")
        # A fresh process avoids lingering speaker state; serialize to keep
        # phrase order canonical. The command line contains no user text.
        with lock:
            try:
                result = subprocess.run(
                    command,
                    input=base64.b64encode(text.encode("utf-8")).decode("ascii"),
                    text=True, capture_output=True, timeout=35, check=False,
                    creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
                )
            except subprocess.TimeoutExpired as error:
                raise RuntimeError("Windows SAPI synthesis timed out") from error
        if result.returncode != 0:
            raise RuntimeError("Windows Russian voice synthesis failed")
        try:
            wav = base64.b64decode(result.stdout.strip(), validate=True)
        except (ValueError, base64.binascii.Error) as error:
            raise RuntimeError("Invalid Windows SAPI WAV encoding") from error
        if len(wav) > 8 * 1024 * 1024 or not validate_echo_wav(wav):
            raise RuntimeError("Windows SAPI returned Echo-incompatible WAV")
        with wave.open(io.BytesIO(wav), "rb") as audio:
            if audio.getnchannels() != 1 or audio.getsampwidth() != 2:
                raise RuntimeError("Windows SAPI must return mono PCM16")
        return wav

    return synthesize


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


if __name__ == "__main__":
    main()
