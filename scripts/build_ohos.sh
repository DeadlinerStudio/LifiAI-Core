#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/dist/ohos"
PACKAGE_NAME="${PACKAGE_NAME:-ffi_ohos}"
LIB_NAME="${LIB_NAME:-ffi_ohos}"
PACKAGE_DIR="${ROOT_DIR}/${PACKAGE_NAME}"
OHOS_TARGETS=(
  "aarch64-unknown-linux-ohos"
  "armv7-unknown-linux-ohos"
  "x86_64-unknown-linux-ohos"
)

find_ohrs() {
  if command -v ohrs >/dev/null 2>&1; then
    command -v ohrs
    return 0
  fi

  if [[ -x "${ROOT_DIR}/target/debug/ohrs" ]]; then
    echo "${ROOT_DIR}/target/debug/ohrs"
    return 0
  fi

  if [[ -x "${ROOT_DIR}/target/release/ohrs" ]]; then
    echo "${ROOT_DIR}/target/release/ohrs"
    return 0
  fi

  return 1
}

has_ohos_sdk() {
  [[ -n "${OHOS_NDK_HOME:-}" || -n "${OHOS_SDK_NATIVE:-}" ]]
}

check_ohos_targets() {
  local installed target missing=()
  installed="$(rustup target list --installed)"

  for target in "${OHOS_TARGETS[@]}"; do
    if ! grep -qx "${target}" <<<"${installed}"; then
      missing+=("${target}")
    fi
  done

  if (( ${#missing[@]} > 0 )); then
    echo "warning: missing OpenHarmony Rust targets: ${missing[*]}" >&2
    echo "hint: run 'rustup target add ${missing[*]}' before building real OpenHarmony artifacts." >&2
    return 1
  fi

  return 0
}

copy_host_validation_artifact() {
  local target_dir lib_ext built_lib validation_dir

  target_dir="${ROOT_DIR}/target/release"
  validation_dir="${OUT_DIR}/host-validation"
  mkdir -p "${validation_dir}"

  case "$(uname -s)" in
    Darwin)
      lib_ext="dylib"
      ;;
    Linux)
      lib_ext="so"
      ;;
    MINGW*|MSYS*|CYGWIN*)
      lib_ext="dll"
      ;;
    *)
      echo "warning: unsupported host platform for artifact copy; skipping local library export." >&2
      return 0
      ;;
  esac

  built_lib="${target_dir}/lib${LIB_NAME}.${lib_ext}"

  if [[ -f "${built_lib}" ]]; then
    cp "${built_lib}" "${validation_dir}/"
    echo "==> Host validation artifact copied to ${validation_dir}"
  else
    echo "warning: expected host validation library not found at ${built_lib}" >&2
  fi
}

mkdir -p "${OUT_DIR}"
cd "${ROOT_DIR}"

if [[ ! -d "${PACKAGE_DIR}" ]]; then
  echo "error: package directory not found: ${PACKAGE_DIR}" >&2
  exit 1
fi

OHRS_BIN="$(find_ohrs || true)"

if [[ -n "${OHRS_BIN}" ]] && has_ohos_sdk && check_ohos_targets; then
  echo "==> Building OpenHarmony package for ${PACKAGE_NAME} with ohrs"
  echo "==> Using reqwest with rustls TLS to avoid target OpenSSL toolchain requirements"
  echo "==> Using ohrs binary: ${OHRS_BIN}"

  cd "${PACKAGE_DIR}"
  "${OHRS_BIN}" build --release --dist "${OUT_DIR}"
  echo "==> ohrs build finished. Check the generated dist output from ohrs."
else
  if [[ -z "${OHRS_BIN}" ]]; then
    echo "warning: ohrs is not installed; falling back to cargo build for shape validation only." >&2
    echo "hint: install it with 'cargo install ohrs' to produce real OpenHarmony artifacts." >&2
  elif ! has_ohos_sdk; then
    echo "warning: OpenHarmony SDK environment is missing; falling back to cargo build validation." >&2
    echo "hint: export OHOS_NDK_HOME or OHOS_SDK_NATIVE and rerun this script for a real OpenHarmony artifact build." >&2
  else
    echo "warning: OpenHarmony Rust targets are incomplete; falling back to cargo build validation." >&2
  fi

  cargo build --release -p "${PACKAGE_NAME}"
  copy_host_validation_artifact
  echo "==> Cargo fallback completed. Install ohrs to produce real Harmony artifacts."
fi

echo "==> Suggested next step: add OHOS-specific napi/ohrs exports inside ffi_ohos only."
