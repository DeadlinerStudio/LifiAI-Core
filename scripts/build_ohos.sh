#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/dist/ohos"
PACKAGE_NAME="${PACKAGE_NAME:-ffi_ohos}"

mkdir -p "${OUT_DIR}"
cd "${ROOT_DIR}"

if command -v ohrs >/dev/null 2>&1; then
  echo "==> Building OpenHarmony package for ${PACKAGE_NAME} with ohrs"
  ohrs build -p "${PACKAGE_NAME}"
  echo "==> ohrs build finished. Check the generated dist output from ohrs."
else
  echo "warning: ohrs is not installed; falling back to cargo build for shape validation only."
  cargo build --release -p "${PACKAGE_NAME}"
  echo "==> Cargo fallback completed. Install ohrs to produce real Harmony artifacts."
fi

echo "==> Suggested next step: add OHOS-specific napi/ohrs exports inside ffi_ohos only."
