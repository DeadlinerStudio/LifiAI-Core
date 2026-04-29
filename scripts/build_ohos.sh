#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/dist/ohos"
PACKAGE_NAME="${PACKAGE_NAME:-ffi_ohos}"
PACKAGE_DIR="${ROOT_DIR}/${PACKAGE_NAME}"
OHOS_TARGETS=(
  "aarch64-unknown-linux-ohos"
  "armv7-unknown-linux-ohos"
  "x86_64-unknown-linux-ohos"
)

resolve_ohos_native_home() {
  local candidate
  local candidates=(
    "${OHOS_NDK_HOME:-}"
    "${OHOS_SDK_NATIVE:-}"
    "${HOS_SDK_HOME:-}/native"
    "${HARMONY_SDK_HOME:-}/native"
  )

  for candidate in "${candidates[@]}"; do
    if [[ -n "${candidate}" && -d "${candidate}" ]]; then
      echo "${candidate}"
      return 0
    fi
  done

  return 1
}

bootstrap_ohos_env() {
  local native_home
  native_home="$(resolve_ohos_native_home || true)"

  if [[ -n "${native_home}" ]]; then
    export OHOS_NDK_HOME="${OHOS_NDK_HOME:-${native_home}}"
    export OHOS_SDK_NATIVE="${OHOS_SDK_NATIVE:-${native_home}}"
  fi
}

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

require_ohrs() {
  local ohrs_bin
  ohrs_bin="$(find_ohrs || true)"

  if [[ -z "${ohrs_bin}" ]]; then
    echo "error: ohrs is required to build Harmony artifacts." >&2
    echo "hint: install it with 'cargo install ohrs --locked'." >&2
    exit 1
  fi

  echo "${ohrs_bin}"
}

require_ohos_sdk() {
  if ! resolve_ohos_native_home >/dev/null 2>&1; then
    echo "error: OpenHarmony SDK environment is missing." >&2
    echo "hint: set OHOS_NDK_HOME, OHOS_SDK_NATIVE, or HOS_SDK_HOME before building." >&2
    exit 1
  fi
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
    echo "error: missing OpenHarmony Rust targets: ${missing[*]}" >&2
    echo "hint: run 'rustup target add ${missing[*]}' before building." >&2
    exit 1
  fi
}

mkdir -p "${OUT_DIR}"
cd "${ROOT_DIR}"

if [[ ! -d "${PACKAGE_DIR}" ]]; then
  echo "error: package directory not found: ${PACKAGE_DIR}" >&2
  exit 1
fi

bootstrap_ohos_env
OHRS_BIN="$(require_ohrs)"
require_ohos_sdk
check_ohos_targets

echo "==> Building OpenHarmony package for ${PACKAGE_NAME} with ohrs"
echo "==> Using reqwest with rustls TLS to avoid target OpenSSL toolchain requirements"
echo "==> Using ohrs binary: ${OHRS_BIN}"
echo "==> Using OpenHarmony native SDK: ${OHOS_NDK_HOME}"

cd "${PACKAGE_DIR}"
"${OHRS_BIN}" build --release --dist "${OUT_DIR}"

echo "==> ohrs build finished. Check the generated dist output from ohrs."
