#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/dist/ohos"
PACKAGE_NAME="${PACKAGE_NAME:-ffi_ohos}"
LIB_NAME="${LIB_NAME:-ffi_ohos}"

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

if command -v ohrs >/dev/null 2>&1 && [[ -n "${OHOS_NDK_HOME:-}" ]]; then
  echo "==> Building OpenHarmony package for ${PACKAGE_NAME} with ohrs"
  echo "==> Using reqwest with rustls TLS to avoid target OpenSSL toolchain requirements"
  ohrs build -p "${PACKAGE_NAME}"
  echo "==> ohrs build finished. Check the generated dist output from ohrs."
else
  if command -v ohrs >/dev/null 2>&1; then
    echo "warning: ohrs is installed but OHOS_NDK_HOME is not set; falling back to cargo build validation." >&2
    echo "hint: export OHOS_NDK_HOME and rerun this script for a real OpenHarmony artifact build." >&2
  else
    echo "warning: ohrs is not installed; falling back to cargo build for shape validation only."
  fi
  cargo build --release -p "${PACKAGE_NAME}"
  copy_host_validation_artifact
  echo "==> Cargo fallback completed. Install ohrs to produce real Harmony artifacts."
fi

echo "==> Suggested next step: add OHOS-specific napi/ohrs exports inside ffi_ohos only."
