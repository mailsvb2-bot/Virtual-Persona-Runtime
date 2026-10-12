"""RT0 native Windows voice: canonical loopback auth and Unicode contract."""
import base64
import http.client
from http.server import ThreadingHTTPServer
import io
import json
from pathlib import Path
import sys
import threading
import unittest
from unittest.mock import patch
import wave

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from metrotrance_tts_bridge import make_handler, validate_echo_wav
import windows_sapi_tts_bridge as bridge
import windows_sapi_prepare as prepare


def wav_fixture():
    out = io.BytesIO()
    with wave.open(out, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(22050)
        w.writeframes(bytes(4400))
    return out.getvalue()


class WindowsSapiContract(unittest.TestCase):
    def test_fixed_script_uses_native_russian_female_and_safe_stdin(self):
        script = bridge._PS_SCRIPT
        self.assertIn('Microsoft Irina Desktop', script)
        self.assertIn('ru-RU', script)
        self.assertIn('Female', script)
        self.assertIn('FromBase64String', script)
        self.assertIn('SetOutputToWaveStream', script)
        self.assertNotIn('Invoke-WebRequest', script)

    def test_private_http_accepts_only_right_token_and_provider(self):
        observed = []
        wav = wav_fixture()
        def synth(text):
            observed.append(text)
            return wav
        token = "test-windows-russian-female-voice-private"
        server = ThreadingHTTPServer(
            ("127.0.0.1", 0),
            make_handler(token, synth, provider_id="windows-sapi"),
        )
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        def request(token_to_use, model):
            conn = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=3)
            try:
                conn.request(
                    "POST", "/v1/audio/speech",
                    json.dumps({"model": model, "voice": "local",
                                "response_format": "wav",
                                "input": "Здравствуйте, я рада вас видеть!"}, ensure_ascii=False)
                    .encode("utf-8"),
                    {"Authorization": "Bearer " + token_to_use,
                     "Content-Type": "application/json"},
                )
                response = conn.getresponse()
                return response.status, response.read()
            finally:
                conn.close()
        try:
            self.assertEqual(request("incorrect", "windows-sapi")[0], 401)
            self.assertEqual(request(token, "piper")[0], 400)
            self.assertEqual(observed, [])
            status, data = request(token, "windows-sapi")
            self.assertEqual(status, 200)
            self.assertEqual(data, wav)
            self.assertTrue(validate_echo_wav(data))
            self.assertEqual(observed, ["Здравствуйте, я рада вас видеть!"])
        finally:
            server.shutdown()
            server.server_close()
            worker.join(timeout=3)



class WindowsSapiPreflightContract(unittest.TestCase):
    """Never replace a working Owner Lab if native speech preflight fails."""

    def test_success_executes_actual_synthesizer_with_bounded_timeout(self):
        python = Path("C:/rt0/venv/Scripts/python.exe")
        with patch.object(prepare.subprocess, "run",
                          return_value=prepare.subprocess.CompletedProcess([], 0)) as run:
            prepare.verify_system_voice(python)
        args, kwargs = run.call_args
        self.assertEqual(args[0][0], str(python))
        self.assertEqual(args[0][1], "-c")
        self.assertIn("make_windows_sapi_synthesizer()", args[0][2])
        self.assertIn("Проверка голоса.", args[0][2])
        self.assertEqual(kwargs["timeout"], 45)
        self.assertFalse(kwargs["check"])
        self.assertEqual(kwargs["stdout"], prepare.subprocess.DEVNULL)
        self.assertEqual(kwargs["stderr"], prepare.subprocess.DEVNULL)

    def test_missing_voice_or_invalid_pcm_fails_closed(self):
        with patch.object(prepare.subprocess, "run",
                          return_value=prepare.subprocess.CompletedProcess([], 1)):
            with self.assertRaisesRegex(RuntimeError, "existing Owner Lab was not stopped"):
                prepare.verify_system_voice(Path("python.exe"))

    def test_hung_synthesis_fails_closed(self):
        with patch.object(prepare.subprocess, "run",
                          side_effect=prepare.subprocess.TimeoutExpired("python.exe", 45)):
            with self.assertRaisesRegex(RuntimeError, "existing Owner Lab was not stopped"):
                prepare.verify_system_voice(Path("python.exe"))

    def test_missing_python_fails_closed(self):
        with patch.object(prepare.subprocess, "run", side_effect=OSError("not found")):
            with self.assertRaisesRegex(RuntimeError, "existing Owner Lab was not stopped"):
                prepare.verify_system_voice(Path("python.exe"))


class PersistentSapiRuntimeContract(unittest.TestCase):
    """Fast repeated speech must reuse one child; failures fail closed."""

    class FakeChild:
        def __init__(self, responses):
            import io as io_module
            self.stdout = io_module.StringIO("".join(responses))
            self.stdin = io_module.StringIO()
            self.terminated = False
            self.killed = 0
            self.environment = {}

        def poll(self):
            return 0 if self.terminated else None

        def kill(self):
            self.killed += 1
            self.terminated = True

        def wait(self, timeout=None):
            self.terminated = True
            return 0

    def test_two_russian_utterances_reuse_one_private_voice_host(self):
        import os
        wav = base64.b64encode(wav_fixture()).decode("ascii") + "\n"
        child = self.FakeChild(["READY\n", wav, wav])
        synth = bridge.WindowsSapiSynthesizer(1)
        try:
            with patch.object(bridge.subprocess, "Popen", return_value=child) as popen:
                self.assertEqual(synth("Здравствуйте."), wav_fixture())
                self.assertEqual(synth("Продолжаю ответ."), wav_fixture())
            popen.assert_called_once()
            env = popen.call_args.kwargs["env"]
            self.assertEqual(env["VPR_WINDOWS_SAPI_RATE"], "1")
            self.assertNotIn("VPR_DID_API_KEY", env)
            self.assertNotIn("VPR_METROTRANCE_TTS_TOKEN", env)
            sent = child.stdin.getvalue().splitlines()
            self.assertEqual([base64.b64decode(x).decode("utf-8") for x in sent],
                             ["Здравствуйте.", "Продолжаю ответ."])
            self.assertEqual(child.killed, 0)
        finally:
            synth.close()

    def test_invalid_inputs_do_not_launch_voice_process(self):
        synth = bridge.WindowsSapiSynthesizer(1)
        try:
            with patch.object(bridge.subprocess, "Popen") as start:
                with self.assertRaises(ValueError):
                    synth(" ")
                with self.assertRaises(ValueError):
                    synth("а" * 4096)
                start.assert_not_called()
        finally:
            synth.close()

    def test_worker_failure_is_bounded_and_later_request_starts_fresh(self):
        wav = base64.b64encode(wav_fixture()).decode("ascii") + "\n"
        failed = self.FakeChild(["READY\n", "ERR\n"])
        recovered = self.FakeChild(["READY\n", wav])
        synth = bridge.WindowsSapiSynthesizer(1)
        try:
            with patch.object(bridge.subprocess, "Popen", side_effect=[failed, recovered]) as spawn:
                with self.assertRaisesRegex(RuntimeError, "unavailable"):
                    synth("Сбой.")
                self.assertTrue(failed.terminated)
                self.assertEqual(synth("Следующая попытка."), wav_fixture())
                self.assertEqual(spawn.call_count, 2)
        finally:
            synth.close()

    def test_missing_ready_never_accepts_private_speech(self):
        failed = self.FakeChild(["UNEXPECTED\n"])
        synth = bridge.WindowsSapiSynthesizer(1)
        try:
            with patch.object(bridge.subprocess, "Popen", return_value=failed):
                with self.assertRaisesRegex(RuntimeError, "unavailable"):
                    synth("Нельзя отправлять.")
                self.assertEqual(failed.stdin.getvalue(), "")
                self.assertEqual(failed.killed, 1)
        finally:
            synth.close()

    def test_sapi_rate_range_explicit(self):
        import os
        with patch.dict(os.environ, {"VPR_WINDOWS_SAPI_RATE": "4"}):
            with self.assertRaises(ValueError):
                bridge._rate()
        with patch.dict(os.environ, {"VPR_WINDOWS_SAPI_RATE": "1"}):
            self.assertEqual(bridge._rate(), 1)
        with self.assertRaises(ValueError):
            bridge.WindowsSapiSynthesizer(-3)

    @unittest.skipUnless(__import__("os").name == "nt", "Native SAPI requires Windows")
    def test_real_native_windows_host_reuses_voice_and_returns_distinct_audio(self):
        voice = bridge.make_windows_sapi_synthesizer()
        try:
            first = voice("Здравствуйте. Рада вас слышать.")
            second = voice("Продолжаю ответ.")
            self.assertTrue(validate_echo_wav(first))
            self.assertTrue(validate_echo_wav(second))
            self.assertNotEqual(first, second)
        finally:
            voice.close()

if __name__ == "__main__":
    unittest.main()
