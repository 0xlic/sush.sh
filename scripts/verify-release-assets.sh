#!/bin/sh
set -eu

tag="${1:-}"
repo="${2:-0xlic/sush.sh}"

if [ -z "$tag" ]; then
  echo "error: missing release tag" >&2
  echo "usage: $0 vX.Y.Z [owner/repo]" >&2
  exit 2
fi

case "$tag" in
  v*) ;;
  *) tag="v$tag" ;;
esac

if ! command -v gh > /dev/null 2>&1; then
  echo "error: gh is required to verify GitHub release assets" >&2
  exit 1
fi

assets=$(
  if ! gh release view "$tag" --repo "$repo" --json assets --jq '.assets[].name' 2> /dev/null; then
    echo "error: release $tag not found in $repo" >&2
    exit 1
  fi
)

if [ -z "$assets" ]; then
  echo "error: release $tag in $repo has no assets" >&2
  exit 1
fi

targets='
aarch64-apple-darwin
x86_64-apple-darwin
aarch64-unknown-linux-gnu
x86_64-unknown-linux-gnu
i686-pc-windows-msvc
x86_64-pc-windows-msvc
'

matched_assets=""
for target in $targets; do
  asset=$(printf '%s\n' "$assets" | grep -F "$target" | head -n 1 || true)
  if [ -z "$asset" ]; then
    echo "error: missing release asset for $target" >&2
    exit 1
  fi
  matched_assets="${matched_assets}${asset}
"
done

checksum_asset=$(printf '%s\n' "$assets" | grep -F 'sha256.sum' | head -n 1 || true)
if [ -z "$checksum_asset" ]; then
  checksum_asset=$(printf '%s\n' "$assets" | grep -Ei 'sha256|checksum' | head -n 1 || true)
fi
if [ -z "$checksum_asset" ]; then
  echo "error: missing checksum asset in release $tag" >&2
  exit 1
fi

tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT

if ! gh release download "$tag" --repo "$repo" --pattern "$checksum_asset" --dir "$tmp_dir" > /dev/null 2>&1; then
  echo "error: failed to download checksum asset $checksum_asset from $repo $tag" >&2
  exit 1
fi

checksum_file="$tmp_dir/$checksum_asset"
if [ ! -f "$checksum_file" ]; then
  echo "error: checksum asset $checksum_asset was not downloaded" >&2
  exit 1
fi

for asset in $matched_assets; do
  if ! grep -F "$asset" "$checksum_file" > /dev/null; then
    echo "error: checksum file $checksum_asset does not include $asset" >&2
    exit 1
  fi
done

echo "release $tag in $repo has all expected assets and checksum coverage"
