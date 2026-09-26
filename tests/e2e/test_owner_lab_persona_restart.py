import json
import os
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PORT = 18887
BASE = f"http://127.0.0.1:{PORT}"
ORIGIN = BASE


def request_json(method: str, path: str, body=None, csrf: str | None = None):
    data = None if body is None else json.dumps(body).encode("utf-8")
    headers = {}
    if data is not None:
        headers["Content-Type"] = "application/json"
    if csrf is not None:
        headers["Origin"] = ORIGIN
        headers["X-VPR-CSRF"] = csrf
    request = urllib.request.Request(
        f"{BASE}{path}",
        data=data,
        headers=headers,
        method=method,
    )
    with urllib.request.urlopen(request, timeout=3) as response:
        return json.loads(response.read().decode("utf-8"))


def wait_ready(process: subprocess.Popen, timeout: float = 15.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"Owner Lab exited early with code {process.returncode}")
        try:
            return request_json("GET", "/api/bootstrap")
        except (urllib.error.URLError, TimeoutError):
            time.sleep(0.1)
    raise RuntimeError("Owner Lab did not become ready")


def start_owner_lab(store_path: Path) -> subprocess.Popen:
    binary = ROOT / "target" / "debug" / ("vpr-owner-lab.exe" if os.name == "nt" else "vpr-owner-lab")
    if not binary.exists():
        raise RuntimeError(f"Owner Lab binary is missing: {binary}")

    env = os.environ.copy()
    env.update(
        {
            "VPR_DID_ENDPOINT": "http://127.0.0.1:9",
            "VPR_DID_API_KEY": "restart-e2e-did-secret",
            "VPR_DID_AGENT_ID": "restart-e2e-agent",
            "VPR_OWNER_LAB_PORT": str(PORT),
            "VPR_OWNER_LAB_PERSONA_STORE_PATH": str(store_path),
        }
    )
    return subprocess.Popen(
        [str(binary)],
        cwd=ROOT,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )


def stop_owner_lab(process: subprocess.Popen) -> None:
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="vpr-persona-restart-") as temp:
        store_path = Path(temp) / "reviewed-persona.json"

        first = start_owner_lab(store_path)
        try:
            bootstrap = wait_ready(first)
            csrf = str(bootstrap["csrf_token"])

            request_json(
                "POST",
                "/api/persona/create",
                {"persona_id": "restart-e2e-owner"},
                csrf,
            )
            for answer in (
                "Owner restart E2E identity",
                "Concise and precise",
                "Verify before claiming success",
            ):
                request_json(
                    "POST",
                    "/api/persona/capture/answer",
                    {"answer": answer},
                    csrf,
                )

            request_json("POST", "/api/persona/capture/finish", {}, csrf)
            capture = request_json("GET", "/api/persona/capture")
            claim_ids = [claim["claim_id"] for claim in capture["claims"]]
            if len(claim_ids) != 3:
                raise AssertionError(f"expected 3 captured claims, got {claim_ids}")

            for claim_id in claim_ids:
                request_json(
                    "POST",
                    "/api/persona/claims/approve",
                    {"claim_id": claim_id},
                    csrf,
                )

            request_json("POST", "/api/persona/review/complete", {}, csrf)
            before = request_json("POST", "/api/persona/reviewed", {}, csrf)
            status_before = request_json("GET", "/api/status")
            if status_before["owner_context_state"] != "reviewed":
                raise AssertionError(status_before)
            if status_before["reviewed_owner_claims"] != 3:
                raise AssertionError(status_before)
            if not store_path.exists():
                raise AssertionError("reviewed Persona store was not created")
        finally:
            stop_owner_lab(first)

        # This is the contract under test: the process is gone. A fresh process must recover
        # the exact reviewed current state from durable storage, without the restart bridge.
        second = start_owner_lab(store_path)
        try:
            bootstrap = wait_ready(second)
            csrf = str(bootstrap["csrf_token"])
            after = request_json("POST", "/api/persona/reviewed", {}, csrf)
            status_after = request_json("GET", "/api/status")

            if after != before:
                raise AssertionError(
                    "reviewed Persona changed across full process restart:\n"
                    f"before={before!r}\nafter={after!r}"
                )
            if status_after["owner_context_state"] != "reviewed":
                raise AssertionError(status_after)
            if status_after["reviewed_owner_claims"] != 3:
                raise AssertionError(status_after)
            if status_after["persona_version"] != status_before["persona_version"]:
                raise AssertionError((status_before, status_after))
        finally:
            stop_owner_lab(second)

    print("Owner Lab Persona full process restart E2E passed.")


if __name__ == "__main__":
    main()
