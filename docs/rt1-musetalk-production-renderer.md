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
# Must equal a reviewed, clean upstream MuseTalk checkout. Runtime and
# offline preflight share the same verification before importing model code.
export RT1_MUSETALK_SOURCE_REVISION=0a89dec45a0192b824e3cf4daf96c239440c5ed8
export RT1_MUSETALK_AVATAR_DIR=/srv/vpr-persona/prepared
# SHA-256 over precisely the prepared avatar inputs, computed on a trusted
# staging host and independently reviewed/pinned BEFORE starting the worker.
# Do not generate this value from the live avatar directory at startup.
export RT1_MUSETALK_AVATAR_SHA256='<64-lowercase-hex-approved-digest>'
# Independently approved digest of all runtime UNet/Whisper/VAE weights
# and their configurations; must be pinned BEFORE loading torch models.
export RT1_MUSETALK_MODELS_SHA256='<64-lowercase-hex-approved-weights-digest>'
# Independently approved hash of *all* installed Python distributions.
# Do not compute or approve this pin during worker startup.
export RT1_MUSETALK_ENVIRONMENT_SHA256='<64-lowercase-hex-approved-environment-snapshot>'
# Offline installation provenance: wheelhouse and independently approved lock.
export RT1_MUSETALK_WHEELHOUSE_DIR=/srv/vpr-wheelhouse
export RT1_MUSETALK_WHEELHOUSE_MANIFEST=/srv/approved/vpr-wheelhouse-lock.json
export RT1_MUSETALK_WHEELHOUSE_MANIFEST_SHA256='<64-lowercase-hex-approved-wheelhouse-lock>'
# MUST equal the canonical reviewed Persona identity/version in Owner Lab.
export RT1_MUSETALK_PERSONA_ID=persona-a
export RT1_MUSETALK_PERSONA_VERSION=3
export RT1_MUSETALK_GPU_ID=0
export RT1_WORKER_TOKEN='(secret, from deployment secret store)'
export RT1_WORKER_DB=/var/lib/vpr/rt1-worker.sqlite3
cd "$RT1_MUSETALK_ROOT"
python /opt/Virtual-Persona-Runtime/scripts/rt1_media_worker_server.py
\`\`\`

The worker now **rechecks the approved 40-hex upstream source commit and
its clean tracked/untracked source tree on each startup**, as well as the
independently pinned model bytes and approved avatar. A preflight pass alone
must not be used as permission to load a subsequently modified checkout.
The pinned upstream revision is a reviewed installation candidate rather
than proof of real CUDA compatibility.

The process must start from MuseTalk's root to resolve the upstream VAE
folder under \`models/sd-vae\`. It binds **loopback only**; use the existing
authenticated TLS proxy and canonical worker operations for external traffic.

## Pinned upstream GPU environment (pre-production recipe)

The upstream MuseTalk Git `main` examined for this integration resolves to
`0a89dec45a0192b824e3cf4daf96c239440c5ed8`. The documented upstream
install uses **Python 3.10**, **torch 2.0.1**, **torchvision 0.15.2**,
**torchaudio 2.0.2** from the CUDA 11.8 wheel index, plus exact versions in
the upstream `requirements.txt` (not the Python 3.12 CPU transport CI
environment). This is an **installation candidate**, not a GPU-validated
fully transitive lockfile or proof of compatibility with newer GPUs/drivers.

Prepare an isolated non-production environment; **do not run a download or
installer against the working RT1 production server**. Do not overwrite an
existing MuseTalk checkout or avatar. Review licenses and the shell commands
before running.

```sh
git clone https://github.com/TMElyralab/MuseTalk.git /opt/MuseTalk
cd /opt/MuseTalk
git checkout --detach 0a89dec45a0192b824e3cf4daf96c239440c5ed8

# Requires an existing conda installation and appropriate NVIDIA driver.
conda create -n vpr-musetalk-v15 python=3.10 -y
conda activate vpr-musetalk-v15
python -m pip install torch==2.0.1 torchvision==0.15.2 \\
  torchaudio==2.0.2 --index-url https://download.pytorch.org/whl/cu118
python -m pip install -r requirements.txt
python -m pip install 'aiortc>=1.10,<2'
python -m pip install -U openmim
mim install mmengine
mim install 'mmcv==2.0.1'
mim install 'mmdet==3.1.0'
mim install 'mmpose==1.1.0'
python -m pip check
python /opt/Virtual-Persona-Runtime/scripts/rt1_musetalk_environment.py
# Emit exact installed Python package versions for separate review.
python /opt/Virtual-Persona-Runtime/scripts/rt1_musetalk_environment.py \
  --emit-snapshot > /trusted-review/new-python-environment-snapshot.json
# After independent approval of package sources, binary wheel hashes and
# the reported environment_snapshot.sha256, pin it outside the image.
export RT1_MUSETALK_ENVIRONMENT_SHA256='<independently-approved-sha256>'
python /opt/Virtual-Persona-Runtime/scripts/rt1_musetalk_environment.py \
  --approved-snapshot-sha256 "$RT1_MUSETALK_ENVIRONMENT_SHA256"
```

This explicitly checks every directly pinned upstream runtime dependency,
including the MMLab trio, before loading CUDA. It intentionally fails on
Python/package drift, and reports that no CUDA acceptance took place. If the
MMLab package installer upgrades another pinned package, **do not bypass
the failure**; resolve the environment on the isolated build host and capture
an approved `pip freeze` and wheel hashes before promotion. A verified, immutable wheelhouse with independently checked binary SHA-256
hashes is still missing, as is GPU acceptance. The new whole-environment
snapshot detects installed package-version drift but **does not** prove
wheel-file content, licensing or CUDA ABI compatibility. Keep GPU/release
promotion blocked until these external artifacts and real hardware results
have been independently verified.

The upstream weight download command obtains more models than the live RT1
renderer needs, including preparation components; run it **only in the
approved offline preparation environment**. Keep all downloaded weights and
prepared face images out of Git. Then run the zero-GPU source/assets
preflight below, followed by actual CUDA and browser evidence.

## Offline wheelhouse integrity and approval (CPU-only)

Before attempting the GPU acceptance script, stage all approved *binary*
wheel files for the pinned Python 3.10 environment in a dedicated immutable
directory. No package download, CUDA import or installation occurs in these
commands. The approval file must live **outside** the wheelhouse; do not
automatically accept a hash returned by the candidate generator.

```sh
export RT1_MUSETALK_WHEELHOUSE_DIR=/srv/vpr-wheelhouse
export RT1_MUSETALK_WHEELHOUSE_MANIFEST=/srv/review/vpr-wheelhouse-lock.json

# On a trusted staging host, produce a candidate manifest without approval.
python /opt/Virtual-Persona-Runtime/scripts/rt1_musetalk_wheelhouse.py \
  --wheelhouse "$RT1_MUSETALK_WHEELHOUSE_DIR" \
  --draft-manifest-output "$RT1_MUSETALK_WHEELHOUSE_MANIFEST"

# Independently review source, licenses, platform tags and every wheel SHA-256.
# Store the approved manifest digest OUTSIDE the wheelhouse and deployed image.
export RT1_MUSETALK_WHEELHOUSE_MANIFEST_SHA256='<independently-approved-sha256>'
python /opt/Virtual-Persona-Runtime/scripts/rt1_musetalk_wheelhouse.py \
  --wheelhouse "$RT1_MUSETALK_WHEELHOUSE_DIR" \
  --manifest "$RT1_MUSETALK_WHEELHOUSE_MANIFEST" \
  --approved-manifest-sha256 "$RT1_MUSETALK_WHEELHOUSE_MANIFEST_SHA256"
```

The verified manifest records exact wheel filenames, lengths and SHA-256
digests and rejects unexpected files, symlinks, missing wheels and changed
bytes. The candidate generator never marks its own output approved. This is
a **software integrity tool**, not proof that the installed packages came
from that wheelhouse, that every CUDA ABI is compatible, or that real GPU
inference works. On the separately approved clean build host, install with
`--no-index --find-links` against this locked wheelhouse, run `pip check`,
and independently approve the installed-distribution snapshot above.
Until that real clean installation is verified, the deployment gate remains
open and production release remains blocked.

## Reproducible offline clean-install candidate (CPU-only)

Once the wheelhouse has been independently reviewed and its manifest SHA-256
approved, the canonical CPU-only installer can validate the entire locked wheel
set **without running any command**:

```sh
python scripts/rt1_musetalk_clean_install.py \
  --wheelhouse "$RT1_MUSETALK_WHEELHOUSE_DIR" \
  --manifest "$RT1_MUSETALK_WHEELHOUSE_MANIFEST" \
  --approved-manifest-sha256 "$RT1_MUSETALK_WHEELHOUSE_MANIFEST_SHA256"
```

On an **isolated clean-build host** with Python 3.10 already installed,
explicitly authorize a *new* virtual environment. This command creates only
the requested previously nonexistent destination, installs each hash-pinned
local wheel with `--no-index --no-deps --require-hashes`, runs `pip check`,
then prints the entire installed-distribution fingerprint for separate review.

```sh
python scripts/rt1_musetalk_clean_install.py \
  --wheelhouse "$RT1_MUSETALK_WHEELHOUSE_DIR" \
  --manifest "$RT1_MUSETALK_WHEELHOUSE_MANIFEST" \
  --approved-manifest-sha256 "$RT1_MUSETALK_WHEELHOUSE_MANIFEST_SHA256" \
  --python-bin /opt/python310/bin/python3.10 \
  --venv /srv/new-isolated-vpr-rt1-venv \
  --execute
```

The installer does **not** fetch any missing package, silently rewrite the
approved wheelhouse, reuse an old environment or approve the fingerprint it
produces. An incomplete install remains blocked and needs a new destination.
A successful install is not evidence of model/CUDA ABI compatibility, real
GPU inference, avatar rights, browser A/V sync or dual-track STOP. Those
remain separate acceptance gates.

## Prepared face integrity gate

A reviewed Persona ID/version by itself is not proof that the operator's
prepared face files remain unchanged. The worker now requires a separate
deployment-pinned digest of the exact files it consumes: `latents.pt`,
`coords.pkl`, `mask_coords.pkl` and all PNGs in `full_imgs/` and `mask/`.
Symlinked entries are rejected. Verify the prepared inputs on a trusted,
isolated staging host, then record the digest in the deployment secret/config
store independently of the avatar directory:

```sh
python - <<'PY'
import sys
sys.path.insert(0, '/opt/Virtual-Persona-Runtime/scripts')
from rt1_musetalk_renderer import prepared_avatar_digest
print(prepared_avatar_digest('/srv/vpr-persona/prepared'))
PY
```

The approved digest must then be set in `RT1_MUSETALK_AVATAR_SHA256`.
It is checked before any local PyTorch model import or unpickling of
prepared avatar files. If any approved file changes, the worker aborts.
Do not recompute the pinned value automatically on production startup:
doing that would make a malicious replacement self-authorizing.

**Security boundary:** SHA-256 pinning detects change against a separately
approved copy, but does not establish the legal identity or consent of the
person depicted. Record consent, ownership, upstream model licensing,
and authorized review outside the asset directory. Lock down filesystem
write access while the worker is running: hashing before deserialization
does not eliminate malicious concurrent file replacement (TOCTOU).

## Approved model-weight integrity gate

The offline GPU preflight and the real renderer now share the same canonical
runtime model-file list: MuseTalk v1.5 UNet checkpoint/configuration, Whisper
checkpoint/configuration/preprocessor, and SD-VAE checkpoint/configuration.

**The complete model/config byte set must be approved independently**, not
only checked for existence. On a trusted isolated staging machine after
obtaining reviewed upstream weights, compute:

```sh
python - <<'PY'
import sys
sys.path.insert(0, '/opt/Virtual-Persona-Runtime/scripts')
from rt1_musetalk_renderer import approved_model_digest
print(approved_model_digest('/opt/MuseTalk'))
PY
```

Store that reviewed digest **outside the model directory** and configure
`RT1_MUSETALK_MODELS_SHA256`. Worker startup fails **before any torch
import/model deserialization** when the pin is absent, malformed or differs
from the installed bytes. A replacement or symlink in any expected model
file is rejected. A digest is an integrity pin against a separately approved
baseline; it does NOT automatically prove weight provenance, licensing or
runtime safety. Never recompute or approve a new digest automatically in
production. Prevent file replacement during/after verification.

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

## Mandatory zero-GPU preflight before server reservation

Run this **on the prepared staging machine before requesting GPU time**.
It never imports torch, runs CUDA, unpickles Persona data or approves
production. It verifies the actual checked-out upstream MuseTalk commit,
tracked source cleanliness, critical public function signatures, the local
UNet / Whisper / VAE file presence and the independently approved Persona
digest. Use the full upstream Git commit, not the floating `main` branch.

```sh
python /opt/Virtual-Persona-Runtime/scripts/rt1_musetalk_offline_preflight.py \\
  --root /opt/MuseTalk \\
  --avatar-dir /srv/vpr-persona/prepared \\
  --expected-revision '<reviewed-upstream-40-hex-commit>' \\
  --approved-avatar-sha256 "$RT1_MUSETALK_AVATAR_SHA256" \
  --approved-models-sha256 "$RT1_MUSETALK_MODELS_SHA256"
```

A pass is only an **offline compatibility check**. The CUDA runtime,
model tensor compatibility, actual fps, memory usage, real browser A/V and
STOP must still be tested separately. Reject a failure; never bypass it by
allowing a diagnostic renderer to authorize a Persona. This preflight does
not establish licensing, consent or safe artifact provenance by itself.

## Single-command GPU-host acceptance (when NVIDIA is actually available)

Use the EXISTING CPU checks, existing renderer and existing CUDA acceptance in
one strict sequence. The runner installs nothing and cannot approve publication.
On an independently approved NVIDIA host, after the isolated environment,
licensed weights, authorized face and all pins have been provisioned:

```sh
# RT1_VIDEO_RENDERER, RT1_MUSETALK_ROOT, RT1_MUSETALK_AVATAR_DIR,
# RT1_MUSETALK_SOURCE_REVISION, RT1_MUSETALK_MODELS_SHA256,
# RT1_MUSETALK_AVATAR_SHA256, RT1_MUSETALK_PERSONA_ID and
# RT1_MUSETALK_PERSONA_VERSION must already be set by deployment config.
mkdir -m 700 -p "$HOME/vpr-rt1-evidence"
bash /opt/Virtual-Persona-Runtime/scripts/rt1_musetalk_hardware_acceptance.sh \\
  "$HOME/vpr-rt1-evidence/musetalk-compute-001.json"
```

Run from the reviewed Python 3.10 environment and with a GPU driver visible
to `nvidia-smi`. Stop immediately if any stage fails: environment audit,
upstream SHA/clean tree, approved model/avatar digests, real CUDA warmup or
50-window worst-case deadline. The output path must not already exist. This
is **only a compute acceptance**; browser RTP synchronization, real visitor
conversation, interruption and verified two-track STOP remain separately
required. Never copy this JSON into a production-publication certificate.

## Real CUDA compute acceptance (not a browser release certificate)

On the GPU host after installing operator-approved weights/assets and pinning
`RT1_MUSETALK_AVATAR_SHA256`, run the production model itself, not a test
renderer:

```sh
cd "$RT1_MUSETALK_ROOT"
python /opt/Virtual-Persona-Runtime/scripts/rt1_musetalk_gpu_acceptance.py \\
  --windows 50 --max-window-ms 400 \\
  --output /var/lib/vpr/evidence/musetalk-gpu-compute-v1.json
```

The command warms the actual renderer and benchmarks fifty distinct 400 ms
PCM inference windows with CUDA synchronization. It writes a *new* JSON file
with processing throughput, worst and p95 latency, GPU type, hashed Persona
identity and prepared-asset digest. A single window exceeding the deadline
fails the command (exit code 2), rather than hiding overload behind an average.
This is a sustained model compute probe, **not** proof of 25 fps delivered
over WebRTC: compare real audio/video playout timestamps and independent STOP
receipts on a physical browser/device before publication. The evidence always
records `production_release_approved: false` to prevent promotion by this
compute-only tool. The command never downloads models or sends video/PCM to
an external provider.

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
