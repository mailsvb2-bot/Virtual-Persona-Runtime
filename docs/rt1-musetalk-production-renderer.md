# RT1 production Persona renderer — MuseTalk v1.5

This worker runs the **real MuseTalk 1.5 GPU inference graph** on authorized
16 kHz mono PCM from the existing VPR TTS egress. It is NOT a second TTS,
LLM, session manager, RT1 sender or authorization service.

## Installation and licensed assets

Obtain the official MuseTalk source from https://github.com/TMElyralab/MuseTalk
and follow its v1.5 environment/model installation instructions. Review its
upstream model, training-data, avatar-image and deployment licenses before
commercial use. Do not commit model weights, faces, profile data or secrets.

Prepare an authorized person's avatar **offline** using the upstream
\`scripts/realtime_inference.py\` preparation mode; the prepared avatar
directory must contain \`latents.pt\`, \`coords.pkl\`, \`mask_coords.pkl\`,
\`full_imgs/*.png\` and \`mask/*.png\`. Preparation is separate from
the live streaming path. Files containing pickle data must come from
trusted operator-controlled storage, never visitor uploads.

Use the GPU image/environment matching your exact pinned upstream MuseTalk
revision, including PyTorch CUDA, aiortc, av, numpy, transformers, cv2 and
Whisper/MuseTalk dependencies. Do not download models at worker startup.

Configure the existing RT1 worker:

\`\`\`sh
export RT1_VIDEO_RENDERER=musetalk-v15
export RT1_MUSETALK_ROOT=/opt/MuseTalk
export RT1_MUSETALK_AVATAR_DIR=/srv/vpr-persona/prepared
# MUST equal the canonical reviewed Persona identity/version in Owner Lab.
export RT1_MUSETALK_PERSONA_ID=persona-a
export RT1_MUSETALK_PERSONA_VERSION=3
export RT1_MUSETALK_GPU_ID=0
export RT1_WORKER_TOKEN='(secret, from deployment secret store)'
export RT1_WORKER_DB=/var/lib/vpr/rt1-worker.sqlite3
cd "$RT1_MUSETALK_ROOT"
python /opt/Virtual-Persona-Runtime/scripts/rt1_media_worker_server.py
\`\`\`

The process must start from MuseTalk's root to resolve the upstream VAE
folder under \`models/sd-vae\`. It binds **loopback only**; use the existing
authenticated TLS proxy and canonical worker operations for external traffic.

## Media behavior

- One loaded real CUDA Whisper / MuseTalk VAE / UNet / positional-encoding
  pipeline, one lock for GPU inference; sender/session ownership stays in the
  existing aiortc backend and durable SQLite lifecycle.
- Per-session bounded **400 ms = 6,400 PCM16 mono samples = 12,800
  bytes** windows. Each window produces ten model-inferred 25 fps face frames;
  video uses 90 kHz RTP timestamps incrementing 3,600 ticks per frame (not
  aiortc's default 30 fps). An idle sender shows the authorized prepared
  frame, not a synthetic face.
- The canonical Rust `speak_pcm` marks its last authenticated fragment
  `final: true` on the **existing** worker PCM endpoint. Only that explicit
  utterance boundary pads the last incomplete model window with PCM silence;
  the streaming windows are never inferred from an idle timer. Without this
  flag, the final 20–380 ms of the owner's authorized reply could remain
  buffered forever. Legacy fragments without `final` retain their previous
  semantics.
- The authorized PCM enters the MuseTalk queue once. Only after its inferred
  face frames are queued does the renderer release that same PCM to the
  existing audio RTP sender. This avoids audible speech getting ahead of lip
  animation during GPU inference. No alternative TTS or browser speak.
- Startup runs a real 400 ms silent-audio GPU inference warmup; missing or
  incompatible model weights/asset tensors abort startup rather than
  announcing a false-positive ready state.
- STOP first fences the track, then requires the GPU producer thread to
  quiesce before the canonical sender certifies audio+video STOP. Timeout
  remains unconfirmed in the durable worker ledger.
- Model failure, unready CUDA, missing/mismatched prepared assets, PCM overflow
  or invalid track denies new admission and/or fails the authorized media
  operation. No artificial successful receipt.

## Remaining release gates

This implementation has hermetic CI and local aiortc tests, **not** a
completed live GPU acceptance. Production publication must remain gated until
the exact model/assets/GPU are installed and all requirements below pass:

1. Measure warmed model load, first frame, PCM-to-first-inferred-frame latency,
   sustained 25 fps throughput, memory headroom and visible lip sync.
2. Compare recorded RTP audio/video frame timing on a separate real browser;
   400 ms inference windows delay *both* matching speech and video until
   frames exist. Measure resulting user-perceived latency and actual A/V
   offset under load before promotion.
3. Test multiple visitors, GPU overload, interruption during CUDA inference,
   model error, forced restart, stale CREATE identity and independent verified
   dual-track STOP. Confirm no escaped CUDA producer after close.
4. Verify owner consent, authorized photo ownership, privacy and model license;
   publish only through the canonical RT1 readiness gate.

This does not resolve unsupported remote D-ID V2 STOP and does not claim the
test-only synthetic E2E as production proof.


## Owner identity / media isolation

This deployment supports **one reviewed Persona per GPU worker**. The worker
operator pins `RT1_MUSETALK_PERSONA_ID` and `RT1_MUSETALK_PERSONA_VERSION` to
the exact prepared avatar. Both canonical readiness and authenticated CREATE
must match the *existing* owner workspace identity; a stale or different
Persona cannot reuse this renderer. Never accept a visitor-supplied Persona ID,
never select an avatar by a client-provided path, and do not claim this is a
multi-tenant per-owner GPU asset registry. A reviewed Persona correction or
asset replacement requires a new explicitly pinned worker deployment before
publication can succeed.
