#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLS_ROOT="${ROOT_DIR}/.cache/harmony-sdk"
DOWNLOAD_ROOT="${TOOLS_ROOT}/downloads"
COMMANDLINE_ZIP="${DOWNLOAD_ROOT}/commandline-tools.zip"
COMMANDLINE_DIR="${TOOLS_ROOT}/command-line-tools"
HWSDK_DIR="${TOOLS_ROOT}/hwsdk"
RELEASE_REPO="${HARMONY_SDK_RELEASE_REPO:-harmonyos-dev/hos-sdk}"
SDK_PACKAGES="${HARMONY_SDK_PACKAGES:-toolchains:9 OpenHarmony/toolchains:9}"

require_command() {
  local command_name="$1"

  if ! command -v "${command_name}" >/dev/null 2>&1; then
    echo "error: required command not found: ${command_name}" >&2
    exit 1
  fi
}

detect_asset_pattern() {
  case "$(uname -s)" in
    Darwin)
      echo 'commandline-tools-mac-.*\.zip'
      ;;
    Linux)
      echo 'commandline-tools-linux-.*\.zip'
      ;;
    *)
      echo "error: unsupported host platform: $(uname -s)" >&2
      exit 1
      ;;
  esac
}

persist_env() {
  local key="$1"
  local value="$2"

  export "${key}=${value}"

  if [[ -n "${GITHUB_ENV:-}" ]]; then
    printf '%s=%s\n' "${key}" "${value}" >> "${GITHUB_ENV}"
  fi
}

resolve_sdkmgr_bin() {
  local sdkmgr_path
  if [[ ! -d "${COMMANDLINE_DIR}" ]]; then
    return 1
  fi

  sdkmgr_path="$(find "${COMMANDLINE_DIR}" -type f -path '*/sdkmanager/bin/sdkmgr' | head -n 1)"

  if [[ -z "${sdkmgr_path}" ]]; then
    echo "error: failed to locate sdkmgr under ${COMMANDLINE_DIR}" >&2
    exit 1
  fi

  echo "${sdkmgr_path}"
}

github_auth_header() {
  local token="${HARMONY_SDK_GITHUB_TOKEN:-${GITHUB_TOKEN:-${GH_TOKEN:-}}}"

  if [[ -n "${token}" ]]; then
    printf 'Authorization: Bearer %s' "${token}"
  fi
}

resolve_asset_url() {
  if [[ -n "${HARMONY_SDK_DOWNLOAD_URL:-}" ]]; then
    echo "${HARMONY_SDK_DOWNLOAD_URL}"
    return 0
  fi

  local auth_header release_json asset_pattern
  asset_pattern="${HARMONY_SDK_ASSET_PATTERN:-$(detect_asset_pattern)}"
  auth_header="$(github_auth_header || true)"

  if [[ -n "${auth_header}" ]]; then
    release_json="$(
      curl -fsSL \
        -H "Accept: application/vnd.github+json" \
        -H "${auth_header}" \
        "https://api.github.com/repos/${RELEASE_REPO}/releases/latest"
    )"
  else
    release_json="$(
      curl -fsSL \
        -H "Accept: application/vnd.github+json" \
        "https://api.github.com/repos/${RELEASE_REPO}/releases/latest"
    )"
  fi

  echo "${release_json}" | jq -r --arg pattern "${asset_pattern}" \
    '.assets[] | select(.name | test($pattern)) | .browser_download_url' | head -n 1
}

download_commandline_tools() {
  local asset_url auth_header
  asset_url="$(resolve_asset_url)"

  if [[ -z "${asset_url}" || "${asset_url}" == "null" ]]; then
    echo "error: failed to resolve Harmony SDK asset URL." >&2
    echo "hint: set HARMONY_SDK_DOWNLOAD_URL or adjust HARMONY_SDK_ASSET_PATTERN." >&2
    exit 1
  fi

  auth_header="$(github_auth_header || true)"

  mkdir -p "${DOWNLOAD_ROOT}"

  if [[ ! -f "${COMMANDLINE_ZIP}" ]]; then
    echo "==> Downloading Harmony command-line tools"
    if [[ -n "${auth_header}" ]]; then
      curl -fL \
        -H "${auth_header}" \
        -o "${COMMANDLINE_ZIP}" \
        "${asset_url}"
    else
      curl -fL \
        -o "${COMMANDLINE_ZIP}" \
        "${asset_url}"
    fi
  else
    echo "==> Reusing cached Harmony command-line tools archive"
  fi
}

install_commandline_tools() {
  local sdkmgr_bin
  sdkmgr_bin="$(resolve_sdkmgr_bin || true)"

  if [[ -n "${sdkmgr_bin}" && -x "${sdkmgr_bin}" ]]; then
    echo "==> Reusing unpacked Harmony command-line tools"
    return 0
  fi

  rm -rf "${COMMANDLINE_DIR}"
  mkdir -p "${COMMANDLINE_DIR}"
  unzip -q "${COMMANDLINE_ZIP}" -d "${COMMANDLINE_DIR}"
  sdkmgr_bin="$(resolve_sdkmgr_bin)"
  chmod +x "${sdkmgr_bin}"
}

install_sdk_packages() {
  local sdkmgr_bin
  sdkmgr_bin="$(resolve_sdkmgr_bin)"

  mkdir -p "${HWSDK_DIR}"

  persist_env "JAVA_TOOL_OPTIONS" "${JAVA_TOOL_OPTIONS:-} -Duser.country=CN"

  pushd "${TOOLS_ROOT}" >/dev/null
  "${sdkmgr_bin}" install ${SDK_PACKAGES} --accept-license
  popd >/dev/null
}

export_sdk_paths() {
  if [[ ! -d "${HWSDK_DIR}" ]]; then
    echo "error: expected installed Harmony SDK at ${HWSDK_DIR}" >&2
    exit 1
  fi

  persist_env "HOS_SDK_HOME" "${HWSDK_DIR}"
  persist_env "HARMONY_SDK_HOME" "${HWSDK_DIR}"
  persist_env "OHOS_SDK_NATIVE" "${HWSDK_DIR}/native"
  persist_env "OHOS_NDK_HOME" "${HWSDK_DIR}/native"

  echo "==> HOS_SDK_HOME=${HWSDK_DIR}"
  echo "==> OHOS_NDK_HOME=${HWSDK_DIR}/native"
}

require_command curl
require_command jq
require_command unzip

download_commandline_tools
install_commandline_tools
install_sdk_packages
export_sdk_paths
