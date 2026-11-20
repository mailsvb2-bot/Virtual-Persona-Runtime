# RT1 independent visitor HTTPS edge — deployment boundary

This release adds a **visitor-only HTTPS edge** on top of the existing
`vpr-owner-lab` process. It **does not** create a second Persona store,
publication owner, STT/LLM executor, media sender, or STOP mechanism.

The owner remains on the **private loopback Owner Lab**. The visitor opens a
different HTTPS origin on their own device. The server-authenticated owner
status may supply `public_visitor_origin`; the browser then makes a link:
`https://visitor.example.org/rt1/visit#invite=<visitor-token>`. The bearer
stays in the fragment, is imported then removed from browser history before
any backend API is called, and is never part of the GET URL or Nginx log path.

## Software controls already wired

- `VPR_RT1_PUBLIC_VISITOR_ORIGIN` is optional and **server-only**. If unset,
  the owner UI preserves the old local invitation. If set to anything other
  than a plain public HTTPS DNS origin, authenticated owner status refuses it.
- `GET /api/rt1/visitor/bootstrap` returns **only** the CSRF token to the
  visitor (no Owner Lab voice profile, evidence mode or other bootstrap state).
- The Nginx example `deploy/rt1-public-visitor.nginx.conf.template` is an
  **explicit allowlist**. Only visitor HTML, RT1 JS/CSS, the existing microphone
  AudioWorklet, the pinned LiveKit client, minimal bootstrap GET and eight
  canonical visitor POST routes are forwarded. Owner Lab UI `/`, `/rt1`, capture, owner APIs, evidence, admin,
  raw bootstrap and all unknown routes are not forwarded.
- Incoming public POSTs require the exact external HTTPS `Origin`; the edge
  then rewrites Host/Origin for the **existing loopback-only** CSRF parser.
  The canonical visitor token, reviewed-version permission, consent, remote
  sender STOP receipts and revocation remain in their existing modules.
- Nginx and Owner Lab are both intended to run on a trusted server;
  `127.0.0.1:8787` must **never** be bound to a public interface.

## Operator-controlled HTTPS staging setup (not automatic production promotion)

1. Obtain a dedicated public DNS hostname, trusted TLS certificate, approved
   rights for Persona/voice/model and a verified real video worker. An existing
   unrelated production site must **not** be repurposed without authorization.
2. In a separately reviewed environment, start Owner Lab with
   `VPR_RT1_LOCAL_EXPERIMENT=true` and the canonical verified voice worker,
   `RT1_WORKER_ENDPOINT`, `RT1_WORKER_TOKEN`, approved model assets, and
   existing Persona persistence and rights/consent checks. **Do not** enable
   diagnostic RTP as a surrogate for Persona video.
3. Replace **all placeholders** in the Nginx template with the approved
   visitor FQDN and TLS file paths. Change the upstream port in every
   occurrence if Owner Lab does not use 8787. Do not install an unrestricted
   `proxy_pass` or expose the owner listener publicly.
4. Run `nginx -t` and stage the config behind the selected HTTPS hostname.
   Set `VPR_RT1_PUBLIC_VISITOR_ORIGIN=https://<approved-fqdn>` in the
   private Owner Lab environment, then restart the authorized process.
5. From an **independent device**, verify:
   - `GET https://<fqdn>/rt1/visit` returns visitor-only HTML
   - `GET https://<fqdn>/api/rt1/visitor/bootstrap` returns only `csrf_token`
   - `GET https://<fqdn>/`, `/rt1` and `/api/bootstrap` do **not** expose the owner application
   - `POST https://<fqdn>/api/rt1/owner/publish` is denied at the edge
   - no-Origin, cross-origin or GET requests to visitor POST routes are denied
   - a valid owner-issued invitation works with independent browser cookies
     and explicit consent, and pause/unpublish revoke both audio and video;
     old visitor tokens cannot reconnect
6. Check actual TLS, CSP, security headers, GET/POST error statuses, request
   limits, RTC connectivity (ICE/TURN), concurrent visitors and evidence on
   the exact deployment. Remote dual-track STOP must be observed rather than
   inferred from a local media-element close.

## Explicit non-claims

This is a **deployment boundary and working cross-origin link generator**,
not an activated public hostname or independent multi-tenant owner service.
The canonical Owner Lab still contains one reviewed Persona workspace.
Actual live Internet end-to-end acceptance requires authorized DNS/TLS,
non-synthetic GPU inference, external provider credentials and independently
reviewed owner/visitor release evidence. **No runtime release gate was
relaxed**, and production release maturity remains unchanged.

## Software checks

```bash
python3 -m unittest discover -s tests/architecture -p 'test_rt1_public_visitor_edge.py' -v
python3 -m unittest discover -s tests/architecture -p 'test_rt1_browser_action_routes.py' -v
node --check crates/vpr-owner-lab/ui/rt1.js
cargo test -p vpr-owner-lab --bin vpr-owner-lab
```
