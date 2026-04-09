#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/dist/android"
CRATE_NAME="${CRATE_NAME:-ffi_uniffi}"
BINDINGS_DIR="${OUT_DIR}/bindings"
LIB_NAME="${LIB_NAME:-ffi_uniffi}"
KOTLIN_PACKAGE="${KOTLIN_PACKAGE:-com.aritxonly.deadliner.lifi}"

if ! command -v cargo-ndk >/dev/null 2>&1 && ! cargo ndk --version >/dev/null 2>&1; then
  echo "error: cargo-ndk is required. Install it with: cargo install cargo-ndk"
  exit 1
fi

mkdir -p "${OUT_DIR}"
mkdir -p "${BINDINGS_DIR}"

echo "==> Building Android shared libraries for ${CRATE_NAME}"
cd "${ROOT_DIR}"

cargo ndk \
  -t armeabi-v7a \
  -t arm64-v8a \
  -t x86_64 \
  -o "${OUT_DIR}/jniLibs" \
  build \
  --release \
  -p "${CRATE_NAME}"

ANDROID_LIB="${OUT_DIR}/jniLibs/arm64-v8a/lib${LIB_NAME}.so"

echo "==> Generating Kotlin bindings from ${ANDROID_LIB}"
cargo run -p uniffi-bindgen -- \
  generate \
  --library "${ANDROID_LIB}" \
  --language kotlin \
  --out-dir "${BINDINGS_DIR}"

echo "==> Rewriting Kotlin package name to ${KOTLIN_PACKAGE}"
python3 - <<'PY' "${BINDINGS_DIR}" "${KOTLIN_PACKAGE}"
from pathlib import Path
import re
import sys

bindings_dir = Path(sys.argv[1])
package_name = sys.argv[2]
pattern = re.compile(r"^package\s+[\w.]+$", re.MULTILINE)

kt_files = sorted(bindings_dir.rglob("*.kt"))
if not kt_files:
    raise SystemExit(f"no Kotlin bindings found in {bindings_dir}")

for path in kt_files:
    text = path.read_text()
    if pattern.search(text):
        text = pattern.sub(f"package {package_name}", text, count=1)
    else:
        text = f"package {package_name}\n\n{text}"
    path.write_text(text)
PY

echo "==> Android artifacts written to ${OUT_DIR}/jniLibs"
echo "==> Kotlin bindings written to ${BINDINGS_DIR}"
