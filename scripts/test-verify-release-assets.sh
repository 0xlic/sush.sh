#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
verify_script="$script_dir/verify-release-assets.sh"
tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT

fake_bin="$tmp_dir/bin"
mkdir -p "$fake_bin"

cat > "$fake_bin/gh" <<'EOF'
#!/bin/sh
set -eu

state_dir="${FAKE_GH_STATE_DIR:?}"
command="${1:-}"
subject="${2:-}"
action="${3:-}"

if [ "$command" != "release" ]; then
  echo "unexpected gh command: $*" >&2
  exit 2
fi

case "$subject $action" in
  "view v1.2.0")
    cat "$state_dir/assets.txt"
    ;;
  "view v9.9.9")
    echo "release not found" >&2
    exit 1
    ;;
  "download v1.2.0")
    shift 3
    pattern=""
    output_dir="."
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --pattern)
          pattern="$2"
          shift 2
          ;;
        --dir)
          output_dir="$2"
          shift 2
          ;;
        --repo)
          shift 2
          ;;
        *)
          shift
          ;;
      esac
    done
    mkdir -p "$output_dir"
    cp "$state_dir/$pattern" "$output_dir/$pattern"
    ;;
  *)
    echo "unexpected gh release command: $*" >&2
    exit 2
    ;;
esac
EOF
chmod +x "$fake_bin/gh"

PATH="$fake_bin:$PATH"
export PATH

write_assets() {
  state_dir="$1"
  cat > "$state_dir/assets.txt" <<EOF
sush-aarch64-apple-darwin.tar.xz
sush-x86_64-apple-darwin.tar.xz
sush-aarch64-unknown-linux-gnu.tar.xz
sush-x86_64-unknown-linux-gnu.tar.xz
sush-i686-pc-windows-msvc.zip
sush-x86_64-pc-windows-msvc.zip
sush.sha256
EOF
}

write_checksum() {
  state_dir="$1"
  cat > "$state_dir/sush.sha256" <<EOF
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  sush-aarch64-apple-darwin.tar.xz
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  sush-x86_64-apple-darwin.tar.xz
cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc  sush-aarch64-unknown-linux-gnu.tar.xz
dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd  sush-x86_64-unknown-linux-gnu.tar.xz
eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee  sush-i686-pc-windows-msvc.zip
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff  sush-x86_64-pc-windows-msvc.zip
EOF
}

run_success() {
  state_dir="$tmp_dir/$1"
  mkdir -p "$state_dir"
  write_assets "$state_dir"
  write_checksum "$state_dir"
  FAKE_GH_STATE_DIR="$state_dir" "$verify_script" "v1.2.0" "owner/repo" > "$state_dir/out" 2>&1
}

run_failure_contains() {
  name="$1"
  expected="$2"
  state_dir="$tmp_dir/$name"
  mkdir -p "$state_dir"
  write_assets "$state_dir"
  write_checksum "$state_dir"
  shift 2
  "$@" "$state_dir"

  if FAKE_GH_STATE_DIR="$state_dir" "$verify_script" "v1.2.0" "owner/repo" > "$state_dir/out" 2>&1; then
    echo "expected failure for $name" >&2
    exit 1
  fi

  grep -F "$expected" "$state_dir/out" > /dev/null
}

run_success "success"

state_dir="$tmp_dir/missing-release"
mkdir -p "$state_dir"
write_assets "$state_dir"
write_checksum "$state_dir"
if FAKE_GH_STATE_DIR="$state_dir" "$verify_script" "v9.9.9" "owner/repo" > "$state_dir/out" 2>&1; then
  echo "expected missing release failure" >&2
  exit 1
fi
grep -F "release v9.9.9 not found in owner/repo" "$state_dir/out" > /dev/null

run_failure_contains "missing-asset" "missing release asset for x86_64-unknown-linux-gnu" \
  sh -c 'grep -v "x86_64-unknown-linux-gnu" "$0/assets.txt" > "$0/assets.new"; mv "$0/assets.new" "$0/assets.txt"'

run_failure_contains "missing-checksum-entry" "checksum file sush.sha256 does not include sush-i686-pc-windows-msvc.zip" \
  sh -c 'grep -v "i686-pc-windows-msvc" "$0/sush.sha256" > "$0/sush.sha256.new"; mv "$0/sush.sha256.new" "$0/sush.sha256"'
