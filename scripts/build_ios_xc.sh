#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/dist/ios"
HEADERS_DIR="${OUT_DIR}/headers"
PACKAGE_NAME="${PACKAGE_NAME:-ffi_uniffi}"
LIB_NAME="${LIB_NAME:-ffi_uniffi}"

mkdir -p "${OUT_DIR}" "${HEADERS_DIR}"

cd "${ROOT_DIR}"

echo "==> Building iOS device library for ${PACKAGE_NAME}"
cargo build --release -p "${PACKAGE_NAME}" --target aarch64-apple-ios

echo "==> Building iOS simulator libraries for ${PACKAGE_NAME}"
cargo build --release -p "${PACKAGE_NAME}" --target aarch64-apple-ios-sim
cargo build --release -p "${PACKAGE_NAME}" --target x86_64-apple-ios

DEVICE_LIB="${ROOT_DIR}/target/aarch64-apple-ios/release/lib${LIB_NAME}.a"
SIM_ARM64_LIB="${ROOT_DIR}/target/aarch64-apple-ios-sim/release/lib${LIB_NAME}.a"
SIM_X64_LIB="${ROOT_DIR}/target/x86_64-apple-ios/release/lib${LIB_NAME}.a"
SIM_UNIVERSAL_LIB="${OUT_DIR}/lib${LIB_NAME}_simulator.a"
XCFRAMEWORK_DIR="${OUT_DIR}/${LIB_NAME}.xcframework"

echo "==> Creating universal simulator archive"
lipo -create \
  -output "${SIM_UNIVERSAL_LIB}" \
  "${SIM_ARM64_LIB}" \
  "${SIM_X64_LIB}"

if command -v xcodebuild >/dev/null 2>&1; then
  echo "==> Creating XCFramework"
  rm -rf "${XCFRAMEWORK_DIR}"
  xcodebuild -create-xcframework \
    -library "${DEVICE_LIB}" -headers "${HEADERS_DIR}" \
    -library "${SIM_UNIVERSAL_LIB}" -headers "${HEADERS_DIR}" \
    -output "${XCFRAMEWORK_DIR}"
  echo "==> iOS XCFramework written to ${XCFRAMEWORK_DIR}"
else
  echo "warning: xcodebuild not found; skipping XCFramework packaging."
  echo "device lib: ${DEVICE_LIB}"
  echo "simulator lib: ${SIM_UNIVERSAL_LIB}"
fi

echo "==> Next step: add UniFFI-generated Swift sources to the Apple wrapper once bindings are re-enabled."
