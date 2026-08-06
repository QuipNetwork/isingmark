#!/usr/bin/env bash
set -euo pipefail

# Fetch released quip miner binaries for local development.
#
# Wraps quip-miner's crates/quip-coordinator/tools/fetch-miners.sh at the same
# git rev this crate depends on (QUIP_MINER_REV in .gitlab-ci.yml / Cargo.toml),
# then checks that each downloaded binary can run on this host.
#
# Usage:
#   tools/fetch-miners.sh [DEST_DIR]   (default: <repo>/miners)
#   tools/fetch-miners.sh --select-version TAG [TAG ...]
#     Network-free: pick the newest qualifying tag from the arguments and print
#     it. Honours MINERS_ALLOW_PRERELEASE. Exit non-zero when none qualify.
#
# Env:
#   MINERS_ALLOW_PRERELEASE  0|1 (default 1). When 0, only stable versions are
#                            candidates. When 1, prereleases are included.
#                            Widens the set; does not prefer prereleases.
#   MINER_SET                auto | cpu | cpu-cuda (upstream default: auto)
#   DRY_RUN                  if set, resolve and print the plan, skip download
#   TARGETARCH               override arch detection (amd64|arm64)
#
# MINERS_TAG is auto-detected: the newest semver present in every package
# registry repo this run requires (see required_repos). An inherited MINERS_TAG
# is ignored with a warning, then overwritten with the resolved value so the
# upstream script cannot see a stale tag.
#
# Semver ordering implemented here (not sort -V; BSD/GNU sort -V both get
# prerelease order wrong relative to semver):
#   - major.minor.patch compared numerically (v0.10.0 > v0.4.0)
#   - release without prerelease outranks the same core with one
#     (v0.3.0 > v0.3.0-rc1)
#   - prerelease identifiers (dot-separated) left to right; pure digits
#     compare numerically; identifiers that share a non-digit prefix and end
#     in digits (rc9 vs rc54) compare on the numeric suffix; otherwise ASCII
# Not implemented: build metadata (+...), versions without a leading v,
# multi-hyphen prerelease forms beyond a single -PRERELEASE suffix.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CI_YML="${REPO_ROOT}/.gitlab-ci.yml"
QUIP_MINER_REPO="${QUIP_MINER_REPO:-https://gitlab.com/quip.network/quip-miner.git}"
GITLAB_API="${GITLAB_API:-https://gitlab.com/api/v4}"

# ---------------------------------------------------------------------------
# MINERS_ALLOW_PRERELEASE
# ---------------------------------------------------------------------------
validate_allow_prerelease() {
  case "${MINERS_ALLOW_PRERELEASE:-1}" in
  0 | 1) ;;
  *)
    echo "error: MINERS_ALLOW_PRERELEASE must be 0 or 1 (got '${MINERS_ALLOW_PRERELEASE}')" >&2
    exit 1
    ;;
  esac
}

# ---------------------------------------------------------------------------
# Semver helpers (shared by --select-version and the real fetch path)
# ---------------------------------------------------------------------------

# True if $1 looks like vMAJOR.MINOR.PATCH or vMAJOR.MINOR.PATCH-PRERELEASE.
is_semver_tag() {
  local t="$1"
  [[ "${t}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]
}

# Echo "major minor patch prerelease" for a well-formed tag. prerelease is empty
# when absent. Caller must pass is_semver_tag first.
parse_semver() {
  local t="$1"
  local core pre=""
  t="${t#v}"
  if [[ "${t}" == *-* ]]; then
    core="${t%%-*}"
    pre="${t#*-}"
  else
    core="${t}"
  fi
  local major minor patch
  IFS=. read -r major minor patch <<<"${core}"
  printf '%s %s %s %s\n' "${major}" "${minor}" "${patch}" "${pre}"
}

is_prerelease() {
  local t="$1"
  [[ "${t}" == *-* ]]
}

# Compare one prerelease identifier. Echo -1, 0, or 1.
cmp_prerelease_id() {
  local a="$1" b="$2"
  local a_num=0 b_num=0
  if [[ "${a}" =~ ^[0-9]+$ ]]; then
    a_num=1
  fi
  if [[ "${b}" =~ ^[0-9]+$ ]]; then
    b_num=1
  fi
  if [[ "${a_num}" -eq 1 && "${b_num}" -eq 1 ]]; then
    if ((10#${a} < 10#${b})); then
      echo -1
    elif ((10#${a} > 10#${b})); then
      echo 1
    else
      echo 0
    fi
    return
  fi
  # Same non-digit prefix + trailing digits: compare the numeric suffix so
  # rc9 < rc54 (plain ASCII would reverse that).
  local a_pref="" a_suf="" b_pref="" b_suf=""
  if [[ "${a}" =~ ^([^0-9][^0-9]*)([0-9]+)$ ]]; then
    a_pref="${BASH_REMATCH[1]}"
    a_suf="${BASH_REMATCH[2]}"
  elif [[ "${a}" =~ ^([0-9]+)$ ]]; then
    a_suf="${BASH_REMATCH[1]}"
  fi
  if [[ "${b}" =~ ^([^0-9][^0-9]*)([0-9]+)$ ]]; then
    b_pref="${BASH_REMATCH[1]}"
    b_suf="${BASH_REMATCH[2]}"
  elif [[ "${b}" =~ ^([0-9]+)$ ]]; then
    b_suf="${BASH_REMATCH[1]}"
  fi
  if [[ -n "${a_suf}" && -n "${b_suf}" && "${a_pref}" == "${b_pref}" && -n "${a_pref}" ]]; then
    if ((10#${a_suf} < 10#${b_suf})); then
      echo -1
    elif ((10#${a_suf} > 10#${b_suf})); then
      echo 1
    else
      echo 0
    fi
    return
  fi
  # Semver: numeric identifier is lower than non-numeric.
  if [[ "${a_num}" -eq 1 && "${b_num}" -eq 0 ]]; then
    echo -1
    return
  fi
  if [[ "${a_num}" -eq 0 && "${b_num}" -eq 1 ]]; then
    echo 1
    return
  fi
  if [[ "${a}" < "${b}" ]]; then
    echo -1
  elif [[ "${a}" > "${b}" ]]; then
    echo 1
  else
    echo 0
  fi
}

# Compare prerelease strings (dot-separated). Empty means no prerelease and is
# not passed here; callers handle release-vs-prerelease first.
cmp_prerelease() {
  local a="$1" b="$2"
  local -a a_ids b_ids
  local IFS=.
  read -r -a a_ids <<<"${a}"
  read -r -a b_ids <<<"${b}"
  local i n_a=${#a_ids[@]} n_b=${#b_ids[@]} n
  if ((n_a < n_b)); then
    n=${n_a}
  else
    n=${n_b}
  fi
  for ((i = 0; i < n; i++)); do
    local c
    c="$(cmp_prerelease_id "${a_ids[i]}" "${b_ids[i]}")"
    if [[ "${c}" != "0" ]]; then
      echo "${c}"
      return
    fi
  done
  if ((n_a < n_b)); then
    echo -1
  elif ((n_a > n_b)); then
    echo 1
  else
    echo 0
  fi
}

# Echo -1 if $1 < $2, 0 if equal, 1 if $1 > $2.
semver_cmp() {
  local left="$1" right="$2"
  local l_maj l_min l_pat l_pre r_maj r_min r_pat r_pre
  read -r l_maj l_min l_pat l_pre <<<"$(parse_semver "${left}")"
  read -r r_maj r_min r_pat r_pre <<<"$(parse_semver "${right}")"
  if ((10#${l_maj} < 10#${r_maj})); then
    echo -1
    return
  fi
  if ((10#${l_maj} > 10#${r_maj})); then
    echo 1
    return
  fi
  if ((10#${l_min} < 10#${r_min})); then
    echo -1
    return
  fi
  if ((10#${l_min} > 10#${r_min})); then
    echo 1
    return
  fi
  if ((10#${l_pat} < 10#${r_pat})); then
    echo -1
    return
  fi
  if ((10#${l_pat} > 10#${r_pat})); then
    echo 1
    return
  fi
  # Same core: no prerelease ranks higher than with prerelease.
  if [[ -z "${l_pre}" && -z "${r_pre}" ]]; then
    echo 0
    return
  fi
  if [[ -z "${l_pre}" && -n "${r_pre}" ]]; then
    echo 1
    return
  fi
  if [[ -n "${l_pre}" && -z "${r_pre}" ]]; then
    echo -1
    return
  fi
  cmp_prerelease "${l_pre}" "${r_pre}"
}

# Select the newest qualifying version from the arguments. Prints the winner
# on stdout. Exit 1 with a clear message when nothing qualifies.
select_newest_version() {
  validate_allow_prerelease
  local allow="${MINERS_ALLOW_PRERELEASE:-1}"
  local -a qualified=()
  local v
  for v in "$@"; do
    if ! is_semver_tag "${v}"; then
      continue
    fi
    if [[ "${allow}" == "0" ]] && is_prerelease "${v}"; then
      continue
    fi
    qualified+=("${v}")
  done
  if [[ ${#qualified[@]} -eq 0 ]]; then
    if [[ "${allow}" == "0" ]]; then
      echo "error: no qualifying version among candidates (MINERS_ALLOW_PRERELEASE=0 excludes prereleases; set MINERS_ALLOW_PRERELEASE=1 to include them)" >&2
    else
      echo "error: no qualifying version among candidates" >&2
    fi
    return 1
  fi
  local best="${qualified[0]}"
  local c
  for v in "${qualified[@]:1}"; do
    c="$(semver_cmp "${v}" "${best}")"
    if [[ "${c}" == "1" ]]; then
      best="${v}"
    fi
  done
  printf '%s\n' "${best}"
}

# ---------------------------------------------------------------------------
# --select-version (network-free; same selection code as the fetch path)
# ---------------------------------------------------------------------------
if [[ "${1:-}" == "--select-version" ]]; then
  shift
  if [[ $# -eq 0 ]]; then
    echo "error: --select-version requires at least one version argument" >&2
    exit 1
  fi
  select_newest_version "$@"
  exit $?
fi

# ---------------------------------------------------------------------------
# Resolve QUIP_MINER_REV and destination (unchanged structure)
# ---------------------------------------------------------------------------
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

validate_allow_prerelease

if [[ -n "${MINERS_TAG:-}" ]]; then
  echo "warning: MINERS_TAG=${MINERS_TAG} is set in the environment but ignored; version is auto-detected from package registries" >&2
fi

if [[ $# -ge 1 ]]; then
  DEST="$1"
else
  DEST="${REPO_ROOT}/miners"
fi
mkdir -p "${DEST}"
DEST="$(cd "${DEST}" && pwd)"

# ---------------------------------------------------------------------------
# Required package repos for this MINER_SET + host
# (mirrors upstream fetch() requirements: cuda optional in auto is not required)
# ---------------------------------------------------------------------------
MINER_SET_RESOLVED="${MINER_SET:-auto}"
os="$(uname -s)"
raw_arch="${TARGETARCH:-$(uname -m)}"
case "${raw_arch}" in
x86_64 | amd64) arch=amd64 ;;
arm64 | aarch64) arch=arm64 ;;
*)
  echo "error: unsupported arch: ${raw_arch}" >&2
  exit 1
  ;;
esac

# Space-separated display names; map to URL-encoded project paths.
required_repos=""
case "${MINER_SET_RESOLVED}" in
cpu)
  required_repos="quip-miner-cpu"
  ;;
cpu-cuda)
  required_repos="quip-miner-cpu quip-miner-cuda"
  ;;
auto)
  case "${os}" in
  Linux)
    required_repos="quip-miner-cpu"
    ;;
  Darwin)
    required_repos="quip-miner-metal quip-miner-cpu"
    ;;
  *)
    echo "error: unsupported OS: ${os}" >&2
    exit 1
    ;;
  esac
  ;;
*)
  echo "error: unknown MINER_SET '${MINER_SET_RESOLVED}' (want: auto|cpu|cpu-cuda)" >&2
  exit 1
  ;;
esac

repo_project_path() {
  case "$1" in
  quip-miner-cpu) echo "quip.network%2Fquip-miner-cpu" ;;
  quip-miner-cuda) echo "quip.network%2Fquip-miner-cuda" ;;
  quip-miner-metal) echo "quip.network%2Fquip-miner-metal" ;;
  *)
    echo "error: unknown repo name: $1" >&2
    return 1
    ;;
  esac
}

# List unique package versions for a display-name repo. Prints one tag per line.
list_repo_versions() {
  local name="$1"
  local proj
  proj="$(repo_project_path "${name}")"
  local url="${GITLAB_API}/projects/${proj}/packages?per_page=100"
  local body
  if ! body="$(curl --fail --silent --show-error --location "${url}")"; then
    echo "error: failed to list packages for ${name} (${url})" >&2
    return 1
  fi
  # Prefer python3 for JSON; fall back to a tight grep of "version" fields.
  if command -v python3 >/dev/null 2>&1; then
    printf '%s' "${body}" | python3 -c '
import json, sys
data = json.load(sys.stdin)
seen = set()
for p in data:
    v = p.get("version") or ""
    if v and v not in seen:
        seen.add(v)
        print(v)
'
  else
    printf '%s' "${body}" | grep -oE '"version"[[:space:]]*:[[:space:]]*"[^"]+"' |
      sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/' |
      awk 'NF && !seen[$0]++'
  fi
}

# ---------------------------------------------------------------------------
# Discover newest common version across required repos
# ---------------------------------------------------------------------------
declare -a repo_names=()
declare -a repo_version_lines=() # parallel to repo_names; space-joined versions

common_candidates=()
first_repo=1
for name in ${required_repos}; do
  repo_names+=("${name}")
  vers=()
  while IFS= read -r line; do
    [[ -n "${line}" ]] || continue
    vers+=("${line}")
  done < <(list_repo_versions "${name}")
  if [[ ${#vers[@]} -eq 0 ]]; then
    echo "error: no package versions listed for required repo ${name}" >&2
    exit 1
  fi
  repo_version_lines+=("${vers[*]}")
  if [[ "${first_repo}" -eq 1 ]]; then
    common_candidates=("${vers[@]}")
    first_repo=0
  else
    next_common=()
    for c in "${common_candidates[@]}"; do
      for v in "${vers[@]}"; do
        if [[ "${c}" == "${v}" ]]; then
          next_common+=("${c}")
          break
        fi
      done
    done
    common_candidates=("${next_common[@]}")
  fi
done

if [[ ${#common_candidates[@]} -eq 0 ]]; then
  echo "error: no version is common to all required package repos for MINER_SET=${MINER_SET_RESOLVED} on ${os}/${arch}" >&2
  local_i=0
  for name in "${repo_names[@]}"; do
    echo "  ${name}: ${repo_version_lines[local_i]}" >&2
    local_i=$((local_i + 1))
  done
  if [[ "${MINERS_ALLOW_PRERELEASE:-1}" == "0" ]]; then
    echo "  note: MINERS_ALLOW_PRERELEASE=0 (stable only); MINERS_ALLOW_PRERELEASE=1 may help when a required repo publishes only prereleases (metal currently has no stable tag)" >&2
  fi
  exit 1
fi

if ! RESOLVED_TAG="$(select_newest_version "${common_candidates[@]}")"; then
  echo "error: no version common to the required repos qualifies under MINERS_ALLOW_PRERELEASE=${MINERS_ALLOW_PRERELEASE:-1}" >&2
  local_i=0
  for name in "${repo_names[@]}"; do
    echo "  ${name}: ${repo_version_lines[local_i]}" >&2
    local_i=$((local_i + 1))
  done
  if [[ "${MINERS_ALLOW_PRERELEASE:-1}" == "0" ]]; then
    echo "  note: stable-only produced an empty candidate set; MINERS_ALLOW_PRERELEASE=1 may help (metal currently publishes only a prerelease)" >&2
  fi
  exit 1
fi

export MINERS_TAG="${RESOLVED_TAG}"

echo "quip-miner rev: ${QUIP_MINER_REV}"
echo "MINERS_TAG:     ${MINERS_TAG}  (auto-detected)"
echo "MINERS_ALLOW_PRERELEASE: ${MINERS_ALLOW_PRERELEASE:-1}"
echo "MINER_SET:      ${MINER_SET_RESOLVED}"
echo "required repos: ${required_repos}"
local_i=0
for name in "${repo_names[@]}"; do
  echo "  ${name} versions: ${repo_version_lines[local_i]}"
  local_i=$((local_i + 1))
done
echo "destination:    ${DEST}"
echo

tmp=""
cleanup() {
  if [[ -n "${tmp}" && -d "${tmp}" ]]; then
    rm -rf "${tmp}"
  fi
}
trap cleanup EXIT

tmp="$(mktemp -d "${TMPDIR:-/tmp}/isingmark-fetch-miners.XXXXXX")"
clone_dir="${tmp}/quip-miner"

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
  echo "DRY_RUN set: resolved MINERS_TAG=${MINERS_TAG} from repos [${required_repos}]; no binaries downloaded; skipping host verification."
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
