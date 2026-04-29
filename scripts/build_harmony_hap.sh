#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/dist/harmony-hap"
APP_DIR="${HARMONY_APP_DIR:-${1:-}}"
PRODUCT_NAME="${HARMONY_PRODUCT:-default}"
HVIGOR_TASK="${HARMONY_HVIGOR_TASK:-assembleHap}"
OHOS_REGISTRY="${OHOS_NPM_REGISTRY:-https://repo.harmonyos.com/npm/}"
HVIGOR_VERSION="${OHOS_HVIGOR_VERSION:-2.4.2}"
HVIGOR_PLUGIN_VERSION="${OHOS_HVIGOR_OHOS_PLUGIN_VERSION:-${HVIGOR_VERSION}}"
SDK_HOME="${HOS_SDK_HOME:-${OHOS_SDK_HOME:-${HARMONY_SDK_HOME:-}}}"

ensure_app_dir() {
  if [[ -z "${APP_DIR}" ]]; then
    echo "error: HARMONY_APP_DIR is required." >&2
    exit 1
  fi

  if [[ ! -d "${ROOT_DIR}/${APP_DIR}" ]]; then
    echo "error: Harmony app directory not found: ${ROOT_DIR}/${APP_DIR}" >&2
    exit 1
  fi

  APP_DIR="${ROOT_DIR}/${APP_DIR}"

  if [[ ! -f "${APP_DIR}/oh-package.json5" && ! -f "${APP_DIR}/package.json" ]]; then
    echo "error: no Harmony/Node package manifest found in ${APP_DIR}" >&2
    exit 1
  fi
}

ensure_sdk_home() {
  if [[ -z "${SDK_HOME}" || ! -d "${SDK_HOME}" ]]; then
    echo "error: Harmony SDK not found. Set HOS_SDK_HOME before building the HAP." >&2
    exit 1
  fi

  export HOS_SDK_HOME="${SDK_HOME}"
}

configure_registry() {
  npm config set @ohos:registry "${OHOS_REGISTRY}"
}

install_js_dependencies() {
  if [[ -f "${APP_DIR}/package-lock.json" ]]; then
    npm ci --prefix "${APP_DIR}"
  elif command -v ohpm >/dev/null 2>&1 && [[ -f "${APP_DIR}/oh-package.json5" ]]; then
    (cd "${APP_DIR}" && ohpm install)
  else
    npm install --prefix "${APP_DIR}"
  fi
}

ensure_hvigor() {
  if [[ -x "${APP_DIR}/node_modules/.bin/hvigor" ]]; then
    return 0
  fi

  npm install --prefix "${APP_DIR}" \
    "@ohos/hvigor@${HVIGOR_VERSION}" \
    "@ohos/hvigor-ohos-plugin@${HVIGOR_PLUGIN_VERSION}"
}

write_local_properties() {
  printf 'hwsdk.dir=%s\n' "${HOS_SDK_HOME}" > "${APP_DIR}/local.properties"
}

collect_hap_outputs() {
  rm -rf "${OUT_DIR}"
  mkdir -p "${OUT_DIR}"

  find "${APP_DIR}" -type f -name '*.hap' -print0 | while IFS= read -r -d '' hap; do
    cp "${hap}" "${OUT_DIR}/"
  done

  if ! find "${OUT_DIR}" -type f -name '*.hap' | grep -q .; then
    echo "error: no .hap artifacts found under ${APP_DIR}" >&2
    exit 1
  fi
}

ensure_app_dir
ensure_sdk_home
configure_registry
install_js_dependencies
ensure_hvigor
write_local_properties

echo "==> Building Harmony HAP from ${APP_DIR}"
(cd "${APP_DIR}" && ./node_modules/.bin/hvigor --mode module -p "product=${PRODUCT_NAME}" "${HVIGOR_TASK}" --no-daemon)

collect_hap_outputs
echo "==> Harmony HAP artifacts written to ${OUT_DIR}"
