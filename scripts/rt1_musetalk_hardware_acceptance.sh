#!/usr/bin/env bash
# One canonical, non-publishing MuseTalk GPU acceptance sequence.
# No installers, model downloads, production mutations or second sender.
set -euo pipefail
umask 077

die() { printf 'RT1 GPU acceptance BLOCKED: %s\n' "$*" >&2; exit 2; }
for key in RT1_VIDEO_RENDERER RT1_MUSETALK_ROOT RT1_MUSETALK_AVATAR_DIR \
  RT1_MUSETALK_SOURCE_REVISION RT1_MUSETALK_AVATAR_SHA256 \
  RT1_MUSETALK_MODELS_SHA256 RT1_MUSETALK_PERSONA_ID RT1_MUSETALK_PERSONA_VERSION
do
  [[ -n "${!key:-}" ]] || die "missing $key"
done
[[ "$RT1_VIDEO_RENDERER" == "musetalk-v15" ]] || die "unsupported renderer"
command -v nvidia-smi >/dev/null 2>&1 || die "no NVIDIA driver utility; no GPU acceptance performed"
nvidia-smi -L >/dev/null || die "NVIDIA GPU not accessible"
command -v python >/dev/null 2>&1 || die "python interpreter absent"
[[ -d "$RT1_MUSETALK_ROOT" && -d "$RT1_MUSETALK_AVATAR_DIR" ]] || die "missing approved model/avatar directories"

# Resolve this script's committed checkout; do not require a second repo copy.
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
[[ -f "$SCRIPT_DIR/rt1_musetalk_gpu_acceptance.py" ]] || die "GPU acceptance entrypoint missing"
[[ -f "$SCRIPT_DIR/rt1_musetalk_offline_preflight.py" ]] || die "offline preflight missing"
[[ -f "$SCRIPT_DIR/rt1_musetalk_environment.py" ]] || die "runtime preflight missing"

# Evidence must be an operator-approved, non-existing destination.
[[ "$#" -eq 1 ]] || die "usage: bash scripts/rt1_musetalk_hardware_acceptance.sh /safe/path/new-evidence.json"
OUT="$1"
[[ "$OUT" = /* ]] || die "evidence path must be absolute"
[[ ! -e "$OUT" && ! -L "$OUT" ]] || die "refusing to overwrite existing evidence"
[[ -d "$(dirname -- "$OUT")" ]] || die "create the evidence directory first; this script will not mutate arbitrary parent paths"

printf 'Checking pinned Python environment (no GPU model execution)\n'
python "$SCRIPT_DIR/rt1_musetalk_environment.py"
printf 'Checking approved MuseTalk source, avatar and weights (no CUDA execution)\n'
python "$SCRIPT_DIR/rt1_musetalk_offline_preflight.py" \
  --root "$RT1_MUSETALK_ROOT" \
  --avatar-dir "$RT1_MUSETALK_AVATAR_DIR" \
  --expected-revision "$RT1_MUSETALK_SOURCE_REVISION" \
  --approved-avatar-sha256 "$RT1_MUSETALK_AVATAR_SHA256" \
  --approved-models-sha256 "$RT1_MUSETALK_MODELS_SHA256"

printf 'Executing real CUDA inference acceptance; no browser release claim\n'
cd -- "$RT1_MUSETALK_ROOT"
python "$SCRIPT_DIR/rt1_musetalk_gpu_acceptance.py" \
  --windows 50 --max-window-ms 400 --output "$OUT"
printf 'Compute evidence written: %s\n' "$OUT"
printf 'STOP: real browser A/V timing and dual-track STOP are separate mandatory gates.\n'
