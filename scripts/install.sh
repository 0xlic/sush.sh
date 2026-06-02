#!/bin/sh
set -eu

repo="0xlic/sush.sh"
version="${1:-${SUSH_VERSION:-latest}}"
install_dir="${SUSH_INSTALL_DIR:-$HOME/.local/bin}"

need_cmd() {
  if ! command -v "$1" > /dev/null 2>&1; then
    echo "error: $1 is required" >&2
    exit 1
  fi
}

checksum_cmd() {
  if command -v shasum > /dev/null 2>&1; then
    printf '%s\n' "shasum -a 256"
    return
  fi

  if command -v sha256sum > /dev/null 2>&1; then
    printf '%s\n' "sha256sum"
    return
  fi

  echo "error: shasum or sha256sum is required" >&2
  exit 1
}

normalize_version() {
  case "$1" in
    latest) printf '%s\n' "latest" ;;
    v*) printf '%s\n' "$1" ;;
    *) printf 'v%s\n' "$1" ;;
  esac
}

target_triple() {
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os:$arch" in
    Darwin:arm64 | Darwin:aarch64)
      printf '%s\n' "aarch64-apple-darwin"
      ;;
    Darwin:x86_64 | Darwin:amd64)
      printf '%s\n' "x86_64-apple-darwin"
      ;;
    Linux:aarch64 | Linux:arm64)
      printf '%s\n' "aarch64-unknown-linux-gnu"
      ;;
    Linux:x86_64 | Linux:amd64)
      printf '%s\n' "x86_64-unknown-linux-gnu"
      ;;
    *)
      echo "error: unsupported platform: $os $arch" >&2
      exit 1
      ;;
  esac
}

download_base_url() {
  normalized="$(normalize_version "$version")"
  if [ "$normalized" = "latest" ]; then
    printf 'https://github.com/%s/releases/latest/download\n' "$repo"
  else
    printf 'https://github.com/%s/releases/download/%s\n' "$repo" "$normalized"
  fi
}

verify_checksum() {
  archive="$1"
  checksum_file="$2"
  asset_name="$(basename "$archive")"
  expected="$(
    awk -v name="$asset_name" '
      {
        candidate = $2
        sub(/^\*/, "", candidate)
        if (candidate == name) {
          print $1
          exit
        }
      }
    ' "$checksum_file"
  )"

  if [ -z "$expected" ]; then
    echo "error: checksum file does not include $asset_name" >&2
    exit 1
  fi

  command="$(checksum_cmd)"
  actual="$($command "$archive" | awk '{ print $1 }')"
  if [ "$actual" != "$expected" ]; then
    echo "error: checksum mismatch for $asset_name" >&2
    exit 1
  fi
}

extract_archive() {
  archive="$1"
  extract_dir="$2"

  case "$archive" in
    *.tar.xz)
      need_cmd tar
      tar -xf "$archive" -C "$extract_dir"
      ;;
    *.zip)
      need_cmd unzip
      unzip -q "$archive" -d "$extract_dir"
      ;;
    *)
      echo "error: unsupported archive format: $(basename "$archive")" >&2
      exit 1
      ;;
  esac
}

need_cmd curl

target="$(target_triple)"
asset="sush-$target.tar.xz"
base_url="$(download_base_url)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

archive="$tmp_dir/$asset"
checksum_file="$tmp_dir/sha256.sum"
extract_dir="$tmp_dir/extract"
mkdir -p "$extract_dir"

curl -fsSL -o "$archive" "$base_url/$asset"
curl -fsSL -o "$checksum_file" "$base_url/sha256.sum"

verify_checksum "$archive" "$checksum_file"
extract_archive "$archive" "$extract_dir"

binary="$(find "$extract_dir" -type f -name sush | head -n 1 || true)"
if [ -z "$binary" ]; then
  echo "error: archive does not contain sush binary" >&2
  exit 1
fi

mkdir -p "$install_dir"
cp "$binary" "$install_dir/sush"
chmod +x "$install_dir/sush"

echo "sush installed to $install_dir/sush"
