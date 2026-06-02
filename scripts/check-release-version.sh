#!/bin/sh
set -eu

tag="${1:-}"
manifest="${2:-Cargo.toml}"

if [ -z "$tag" ]; then
  echo "error: missing release tag" >&2
  echo "usage: $0 vX.Y.Z [Cargo.toml]" >&2
  exit 2
fi

case "$tag" in
  v*) ;;
  *)
    echo "error: tag must start with v: $tag" >&2
    exit 1
    ;;
esac

tag_version="${tag#v}"
if ! printf '%s\n' "$tag_version" | grep -E '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$' > /dev/null; then
  echo "error: tag must be SemVer with major.minor.patch: $tag" >&2
  exit 1
fi

manifest_version=$(
  awk '
    /^\[package\]/ { in_package = 1; next }
    /^\[/ { in_package = 0 }
    in_package && $1 == "version" {
      value = $3
      gsub(/^"/, "", value)
      gsub(/"$/, "", value)
      print value
      exit
    }
  ' "$manifest"
)

if [ -z "$manifest_version" ]; then
  echo "error: package.version not found in $manifest" >&2
  exit 1
fi

if [ "$tag_version" != "$manifest_version" ]; then
  echo "error: tag version $tag_version does not match Cargo.toml version $manifest_version" >&2
  exit 1
fi

echo "release tag $tag matches Cargo.toml version $manifest_version"
