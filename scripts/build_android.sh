#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/dist/android"
PACKAGE_NAME="${PACKAGE_NAME:-ffi_uniffi}"
BINDINGS_DIR="${OUT_DIR}/bindings"
LIB_NAME="${LIB_NAME:-ffi_uniffi}"

if ! command -v cargo-ndk >/dev/null 2>&1 && ! cargo ndk --version >/dev/null 2>&1; then
  echo "error: cargo-ndk is required. Install it with: cargo install cargo-ndk"
  exit 1
fi

mkdir -p "${OUT_DIR}"
mkdir -p "${BINDINGS_DIR}"

echo "==> Building Android shared libraries for ${PACKAGE_NAME}"
cd "${ROOT_DIR}"

cargo ndk \
  -t armeabi-v7a \
  -t arm64-v8a \
  -t x86_64 \
  -o "${OUT_DIR}/jniLibs" \
  build \
  --release \
  -p "${PACKAGE_NAME}"

ANDROID_LIB="${OUT_DIR}/jniLibs/arm64-v8a/lib${LIB_NAME}.so"

echo "==> Generating Kotlin bindings from ${ANDROID_LIB}"
cargo run -p uniffi-bindgen -- \
  generate \
  --library "${ANDROID_LIB}" \
  --language kotlin \
  --out-dir "${BINDINGS_DIR}"

echo "==> Android artifacts written to ${OUT_DIR}/jniLibs"
echo "==> Kotlin bindings written to ${BINDINGS_DIR}"
