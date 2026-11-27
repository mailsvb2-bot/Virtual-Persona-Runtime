# D-ID V2 LiveKit — real remote STOP evidence procedure

**Scope: non-promoting, exact-candidate observation.** CI fixture, local Echo
disconnect, and browser room disconnect are NOT server-side D-ID STOP receipts.

## Prerequisites
- Authorized staging D-ID account and a session created by the **existing**
  canonical Owner Lab/Echo route, not by an independent demo application.
- A distinct real browser visitor connected and receiving both audio and video.
- Exact code commit and provider configuration fingerprint recorded without secrets.
- A reviewer recording receiver-side media activity before/after owner revoke.
- Run probe on an authorized machine, with API key in environment; never paste
  the key into a console command, terminal recording, or GitHub.

## Read-only observation
While owner/visitor are connected, start:

```bash
python3 scripts/rt1_did_v2_remote_stop_observe.py --session-id "$DID_SESSION_ID" --samples 20 --interval 5 --output did-stop-observation.json
```

During the observation interval, revoke owner publication through the canonical
Owner Lab route. Capture the independent visitor's receiver-side RTP/audio/video
activity immediately before and after revoke, including any stale reconnect
attempts; capture server STOP and late-CREATE outcomes separately. Keep raw
provider details and recordings private, then export sanitized evidence.

The observer calls **GET /v2/agents/sessions/{sessionId}**. Its HTTP transport **refuses redirects** so Basic credentials cannot follow provider-controlled locations. It redacts all provider
response bodies except a small status allowlist; it exits with code 3
(`REMOTE_VIDEO_STOP_UNCONFIRMED`) even when the provider reports `closed`.
A GET response alone is **not** a verified, causally linked termination receipt.
D-ID now documents an exact-session `DELETE /v2/agents/sessions/{sessionId}`
with HTTP `204 Deleted` (<https://docs.d-id.com/reference/deletesession>).
This **permanently deletes** provider-side session data including transcript,
insights and recording; it does not document a bounded time for terminating
both remote media publishers, repeated DELETE after a lost 204, or whether
HTTP 404 means a prior STOP succeeded. Accordingly the canonical D-ID adapter
may issue it only with the explicit operator setting
`VPR_DID_V2_SESSION_DELETE_PERMANENTLY=true` (default false).
This setting changes the existing `close_session` path, not a second STOP
executor. Even a received 204 still returns unconfirmed from the adapter,
and does not open the RT1 release gate. **Never enable on a production account
without approved data deletion/retention policy and an independent receiver
capturing real RTP activity.** A read-only GET remains observation only.

## Release decision
- **Pass local/media access revoke** only with measured independent receiver RTP
  cessation, stale token/SDP denial and server-owned sender cleanup.
- **Pass D-ID remote video termination** only with authenticated provider
  termination evidence linked to the same session, or a verified provider-side
  capability supporting explicit termination. Inactivity timeout is not an
  immediate STOP acknowledgement.
- **Pass RT0 exit** only after canonical exact-candidate evidence evaluator and
  real owner/non-owner, playback, A/V, billing, rights, voice and human gates.
- Otherwise keep RT1 publication blocked and #475 Draft. The observer cannot alter this decision.

Run `python3 -m unittest tests/e2e/test_rt1_did_v2_remote_stop_observe.py`
to check redaction and no-credentials fail-closed behavior.
