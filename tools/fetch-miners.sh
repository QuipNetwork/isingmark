#!/usr/bin/env bash
set -euo pipefail

# Fetch released quip miner binaries for local development.
#
# Wraps quip-miner's crates/quip-coordinator/tools/fetch-miners.sh at the same
# git rev this crate depends on (QUIP_MINER_REV in .gitlab-ci.yml / Cargo.toml),
# then checks that each downloaded binary can run on this host.
#
# Usage: tools/fetch-miners.sh [DEST_DIR]   (default: <repo>/miners)
#
# Env (passed through to the upstream script):
#   MINERS_TAG  package-registry tag (default here: v0.3.0-rc1)
#   MINER_SET   auto | cpu | cpu-cuda (upstream default: auto)
#   DRY_RUN     if set, print the plan and exit without downloading
#   TARGETARCH  override arch detection (amd64|arm64)
#
# Asset names encode arch only, not OS. The cpu miners published for arm64 are
# Linux ELFs; on macOS they download but cannot execute. Metal miners are Mach-O.
# Binaries that fail the host probe are moved to DEST/unusable/ so DEST itself
# is safe to put on PATH (only binaries that actually run remain there).

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CI_YML="${REPO_ROOT}/.gitlab-ci.yml"
QUIP_MINER_REPO="${QUIP_MINER_REPO:-https://gitlab.com/quip.network/quip-miner.git}"

# Prefer the CI variable so the pin stays in one place with Cargo.toml.
if [[ ! -f "${CI_YML}" ]]; then
  echo "error: missing ${CI_YML}" >&2
  exit 1
fi
QUIP_MINER_REV="$(
  sed -n 's/^[[:space:]]*QUIP_MINER_REV:[[:space:]]*"\([0-9a-f]\{7,40\}\)".*/\1/p' "${CI_YML}" |
    head -n 1
)"
if [[ -z "${QUIP_MINER_REV}" ]]; then
  echo "error: could not parse QUIP_MINER_REV from ${CI_YML}" >&2
  exit 1
fi

# CI pins MINERS_TAG=v0.3.0; this wrapper defaults to v0.3.0-rc1. The upstream
# script at the pinned rev also defaults to v0.3.0, so we export MINERS_TAG
# explicitly rather than relying on that upstream default. Override with
# MINERS_TAG=... when you need a different package-registry tag.
export MINERS_TAG="${MINERS_TAG:-v0.3.0-rc1}"

if [[ $# -ge 1 ]]; then
  DEST="$1"
else
  DEST="${REPO_ROOT}/miners"
fi
mkdir -p "${DEST}"
DEST="$(cd "${DEST}" && pwd)"

tmp=""
cleanup() {
  if [[ -n "${tmp}" && -d "${tmp}" ]]; then
    rm -rf "${tmp}"
  fi
}
trap cleanup EXIT

tmp="$(mktemp -d "${TMPDIR:-/tmp}/isingmark-fetch-miners.XXXXXX")"
clone_dir="${tmp}/quip-miner"

echo "quip-miner rev: ${QUIP_MINER_REV}"
echo "MINERS_TAG:     ${MINERS_TAG}"
echo "MINER_SET:      ${MINER_SET:-auto (upstream default if unset)}"
echo "destination:    ${DEST}"
echo

git clone --filter=blob:none --no-checkout "${QUIP_MINER_REPO}" "${clone_dir}"
git -C "${clone_dir}" fetch --depth 1 origin "${QUIP_MINER_REV}"
git -C "${clone_dir}" checkout "${QUIP_MINER_REV}"

upstream="${clone_dir}/crates/quip-coordinator/tools/fetch-miners.sh"
if [[ ! -f "${upstream}" ]]; then
  echo "error: upstream fetch-miners.sh missing at ${upstream}" >&2
  exit 1
fi

bash "${upstream}" "${DEST}"

if [[ -n "${DRY_RUN:-}" ]]; then
  echo
  echo "DRY_RUN set: no binaries downloaded; skipping host verification."
  exit 0
fi

# Known miner binary basenames the upstream script may place in DEST.
# Dwave is a Python wheel (upstream only prints a pip install hint).
miner_names=(
  quip-cpu-sa
  quip-cpu-gibbs
  quip-metal-sa
  quip-metal-gibbs
  quip-cuda-sa
  quip-cuda-gibbs
)

echo
echo "Verifying binaries on this host ($(uname -s) $(uname -m))..."
echo "Probe: --capabilities (same flag as src/backend/registry.rs)"
echo

usable=0
cpu_sa_usable=0
found_any=0
unusable_dir="${DEST}/unusable"
moved_unusable=0

move_unusable() {
  local name="$1"
  local bin="$2"
  mkdir -p "${unusable_dir}"
  # Overwrite any prior copy of the same name (idempotent reruns).
  mv -f "${bin}" "${unusable_dir}/${name}"
  moved_unusable=$((moved_unusable + 1))
  echo "  -> moved to ${unusable_dir}/${name} (kept for inspection; not on PATH)"
}

for name in "${miner_names[@]}"; do
  bin="${DEST}/${name}"
  if [[ ! -e "${bin}" ]]; then
    continue
  fi
  found_any=1
  ftype="$(file -b "${bin}" 2>/dev/null || echo "unknown")"

  if [[ ! -x "${bin}" ]]; then
    echo "${name}: NOT usable (not executable; file: ${ftype})"
    move_unusable "${name}" "${bin}"
    continue
  fi

  # Run the registry probe. set -e is suspended for the failing command
  # inside this if-test. Capture stderr+stdout for the failure reason.
  probe_out=""
  if probe_out="$("${bin}" --capabilities 2>&1)"; then
    echo "${name}: usable (--capabilities ok; file: ${ftype})"
    usable=$((usable + 1))
    if [[ "${name}" == "quip-cpu-sa" ]]; then
      cpu_sa_usable=1
    fi
  else
    probe_status=$?
    reason="--capabilities exited ${probe_status}"
    # Wrong OS/arch typically yields 126 and "cannot execute binary file".
    if [[ "${probe_status}" -eq 126 ]] ||
      [[ "${probe_out}" == *"cannot execute"* ]] ||
      [[ "${probe_out}" == *"Exec format error"* ]]; then
      reason="wrong OS/arch for this host (cannot execute)"
    elif [[ -n "${probe_out}" ]]; then
      # Collapse to one line for the summary.
      reason="$(printf '%s' "${probe_out}" | tr '\n' ' ' | sed 's/[[:space:]]\{1,\}/ /g')"
    fi
    echo "${name}: NOT usable (${reason}; file: ${ftype})"
    move_unusable "${name}" "${bin}"
  fi
done

if [[ "${found_any}" -eq 0 ]]; then
  echo "No miner binaries found under ${DEST}." >&2
  exit 1
fi

echo
echo "Next steps:"
echo "  export PATH=\"${DEST}:\${PATH}\""
echo
echo "${DEST} contains only binaries that probe successfully on this host."
if [[ "${moved_unusable}" -gt 0 ]]; then
  echo "Unusable binaries (${moved_unusable}) were moved to ${unusable_dir}/"
  echo "(kept for inspection or use in a container; not on PATH)."
fi
echo

if [[ "${usable}" -eq 0 ]]; then
  echo "No usable miner binary on this host." >&2
  echo "Real-miner tests still need ISINGMARK_ALLOW_MISSING_MINERS=1." >&2
  exit 1
fi

if [[ "${cpu_sa_usable}" -eq 1 ]]; then
  echo "quip-cpu-sa is usable. Real-miner tests can run without ISINGMARK_ALLOW_MISSING_MINERS."
else
  echo "quip-cpu-sa is absent or not usable on this host."
  echo "tests/session_conformance.rs::quip_cpu_sa_conformance still needs:"
  echo "  ISINGMARK_ALLOW_MISSING_MINERS=1"
  echo "Other backends that probed successfully can still be used from ${DEST}."
fi

echo "done: ${usable} usable binary(ies) in ${DEST}"
