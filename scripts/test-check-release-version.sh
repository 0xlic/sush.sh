#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
check_script="$script_dir/check-release-version.sh"
tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT

write_manifest() {
  version="$1"
  cat > "$tmp_dir/Cargo.toml" <<EOF
[package]
name = "sush"
version = "$version"
edition = "2024"
EOF
}

assert_success() {
  tag="$1"
  version="$2"

  write_manifest "$version"
  "$check_script" "$tag" "$tmp_dir/Cargo.toml" > "$tmp_dir/out" 2>&1
}

assert_failure_contains() {
  tag="$1"
  version="$2"
  expected="$3"

  write_manifest "$version"
  if "$check_script" "$tag" "$tmp_dir/Cargo.toml" > "$tmp_dir/out" 2>&1; then
    echo "expected failure for $tag against $version" >&2
    exit 1
  fi

  grep -F "$expected" "$tmp_dir/out" > /dev/null
}

assert_success "v1.2.0" "1.2.0"
assert_success "v1.2.0-beta.1" "1.2.0-beta.1"
assert_failure_contains "v1.2.0" "1.1.0" "tag version 1.2.0 does not match Cargo.toml version 1.1.0"
assert_failure_contains "1.2.0" "1.2.0" "tag must start with v"
assert_failure_contains "v1.2" "1.2.0" "tag must be SemVer"
assert_failure_contains "v1.2.0.1" "1.2.0.1" "tag must be SemVer"
