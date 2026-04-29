#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLS_ROOT="${ROOT_DIR}/.cache/harmony-sdk"
DOWNLOAD_ROOT="${TOOLS_ROOT}/downloads"
COMMANDLINE_ZIP="${DOWNLOAD_ROOT}/commandline-tools.zip"
COMMANDLINE_DIR="${TOOLS_ROOT}/command-line-tools"
HWSDK_DIR="${TOOLS_ROOT}/hwsdk"
FULL_SDK_EXTRACT_DIR="${TOOLS_ROOT}/full-sdk"
RELEASE_REPO="${HARMONY_SDK_RELEASE_REPO:-harmonyos-dev/hos-sdk}"
SDK_PACKAGES="${HARMONY_SDK_PACKAGES:-toolchains:9 OpenHarmony/toolchains:9}"

require_command() {
  local command_name="$1"

  if ! command -v "${command_name}" >/dev/null 2>&1; then
    echo "error: required command not found: ${command_name}" >&2
    exit 1
  fi
}

detect_commandline_tools_pattern() {
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

detect_full_sdk_pattern() {
  case "$(uname -s)" in
    Darwin)
      echo '(hwsdk|sdk).*(mac|darwin).*(zip|tar\.gz|tgz)$'
      ;;
    Linux)
      echo '(hwsdk|sdk).*(linux).*(zip|tar\.gz|tgz)$'
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

fetch_release_json() {
  local auth_header
  auth_header="$(github_auth_header || true)"

  if [[ -n "${auth_header}" ]]; then
    curl -fsSL \
      -H "Accept: application/vnd.github+json" \
      -H "${auth_header}" \
      "https://api.github.com/repos/${RELEASE_REPO}/releases/latest"
  else
    curl -fsSL \
      -H "Accept: application/vnd.github+json" \
      "https://api.github.com/repos/${RELEASE_REPO}/releases/latest"
  fi
}

resolve_release_asset_url() {
  local asset_pattern="$1"
  local release_json
  release_json="$(fetch_release_json)"

  echo "${release_json}" | jq -r --arg pattern "${asset_pattern}" \
    '.assets[] | select(.name | test($pattern; "i")) | .browser_download_url' | head -n 1
}

download_file() {
  local url="$1"
  local output_path="$2"
  local auth_header
  auth_header="$(github_auth_header || true)"

  mkdir -p "${DOWNLOAD_ROOT}"

  if [[ -n "${auth_header}" ]]; then
    curl -fL -H "${auth_header}" -o "${output_path}" "${url}"
  else
    curl -fL -o "${output_path}" "${url}"
  fi
}

archive_path_for_url() {
  local url="$1"
  local filename
  filename="$(basename "${url%%\?*}")"

  if [[ -z "${filename}" || "${filename}" == "/" ]]; then
    filename="downloaded-archive.zip"
  fi

  echo "${DOWNLOAD_ROOT}/${filename}"
}

resolve_full_sdk_url() {
  if [[ -n "${HARMONY_SDK_FULL_DOWNLOAD_URL:-}" ]]; then
    echo "${HARMONY_SDK_FULL_DOWNLOAD_URL}"
    return 0
  fi

  local asset_pattern asset_url
  asset_pattern="${HARMONY_SDK_FULL_ASSET_PATTERN:-$(detect_full_sdk_pattern)}"
  asset_url="$(resolve_release_asset_url "${asset_pattern}")"

  if [[ -n "${asset_url}" && "${asset_url}" != "null" ]]; then
    echo "${asset_url}"
  fi
}

download_commandline_tools() {
  local asset_url asset_pattern
  asset_pattern="${HARMONY_SDK_ASSET_PATTERN:-$(detect_commandline_tools_pattern)}"
  asset_url="${HARMONY_SDK_DOWNLOAD_URL:-$(resolve_release_asset_url "${asset_pattern}")}"

  if [[ -z "${asset_url}" || "${asset_url}" == "null" ]]; then
    echo "error: failed to resolve Harmony command-line tools asset URL." >&2
    echo "hint: set HARMONY_SDK_DOWNLOAD_URL or adjust HARMONY_SDK_ASSET_PATTERN." >&2
    exit 1
  fi

  if [[ ! -f "${COMMANDLINE_ZIP}" ]]; then
    echo "==> Downloading Harmony command-line tools"
    download_file "${asset_url}" "${COMMANDLINE_ZIP}"
  else
    echo "==> Reusing cached Harmony command-line tools archive"
  fi
}

extract_archive() {
  local archive_path="$1"
  local target_dir="$2"

  rm -rf "${target_dir}"
  mkdir -p "${target_dir}"

  case "${archive_path}" in
    *.zip)
      unzip -q "${archive_path}" -d "${target_dir}"
      ;;
    *.tar.gz|*.tgz)
      tar -xzf "${archive_path}" -C "${target_dir}"
      ;;
    *)
      echo "error: unsupported archive format: ${archive_path}" >&2
      exit 1
      ;;
  esac
}

install_commandline_tools() {
  local sdkmgr_bin
  sdkmgr_bin="$(resolve_sdkmgr_bin || true)"

  if [[ -n "${sdkmgr_bin}" && -x "${sdkmgr_bin}" ]]; then
    echo "==> Reusing unpacked Harmony command-line tools"
    return 0
  fi

  extract_archive "${COMMANDLINE_ZIP}" "${COMMANDLINE_DIR}"
  sdkmgr_bin="$(resolve_sdkmgr_bin)"
  chmod +x "${sdkmgr_bin}"
}

install_sdk_packages_via_sdkmgr() {
  local sdkmgr_bin
  sdkmgr_bin="$(resolve_sdkmgr_bin)"

  mkdir -p "${HWSDK_DIR}"
  persist_env "JAVA_TOOL_OPTIONS" "${JAVA_TOOL_OPTIONS:-} -Duser.country=CN"

  pushd "${TOOLS_ROOT}" >/dev/null
  "${sdkmgr_bin}" install ${SDK_PACKAGES} --accept-license
  popd >/dev/null
}

find_hwsdk_root() {
  local search_root="$1"
  local native_dir

  native_dir="$(find "${search_root}" -type d -path '*/native' | head -n 1)"
  if [[ -z "${native_dir}" ]]; then
    return 1
  fi

  dirname "${native_dir}"
}

install_full_sdk_if_available() {
  local full_sdk_url full_sdk_root full_sdk_archive
  full_sdk_url="$(resolve_full_sdk_url || true)"

  if [[ -z "${full_sdk_url}" ]]; then
    return 1
  fi

  echo "==> Downloading mirrored Harmony SDK archive"
  full_sdk_archive="$(archive_path_for_url "${full_sdk_url}")"
  download_file "${full_sdk_url}" "${full_sdk_archive}"
  extract_archive "${full_sdk_archive}" "${FULL_SDK_EXTRACT_DIR}"

  full_sdk_root="$(find_hwsdk_root "${FULL_SDK_EXTRACT_DIR}" || true)"
  if [[ -z "${full_sdk_root}" ]]; then
    echo "warning: mirrored Harmony SDK archive did not contain a detectable native SDK; falling back to sdkmgr." >&2
    return 1
  fi

  rm -rf "${HWSDK_DIR}"
  mkdir -p "$(dirname "${HWSDK_DIR}")"
  cp -R "${full_sdk_root}" "${HWSDK_DIR}"
  echo "==> Using mirrored Harmony SDK from release asset"
  return 0
}

export_sdk_paths() {
  if [[ ! -d "${HWSDK_DIR}/native" ]]; then
    echo "error: expected installed Harmony SDK native directory at ${HWSDK_DIR}/native" >&2
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
require_command tar

if ! install_full_sdk_if_available; then
  download_commandline_tools
  install_commandline_tools
  install_sdk_packages_via_sdkmgr
fi

export_sdk_paths
