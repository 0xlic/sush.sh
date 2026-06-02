#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
install_script="$script_dir/install.sh"
tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT

fake_bin="$tmp_dir/bin"
mkdir -p "$fake_bin"

cat > "$fake_bin/uname" <<'EOF'
#!/bin/sh
case "${1:-}" in
  -s) printf '%s\n' "${FAKE_UNAME_S:-Darwin}" ;;
  -m) printf '%s\n' "${FAKE_UNAME_M:-arm64}" ;;
  *) printf '%s\n' "${FAKE_UNAME_S:-Darwin}" ;;
esac
EOF

cat > "$fake_bin/curl" <<'CURL_EOF'
#!/bin/sh
set -eu

output=""
url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      output="$2"
      shift 2
      ;;
    -*)
      shift
      ;;
    *)
      url="$1"
      shift
      ;;
  esac
done

if [ -z "$output" ] || [ -z "$url" ]; then
  echo "fake curl expected -o output and url" >&2
  exit 2
fi

printf '%s\n' "$url" >> "$FAKE_CURL_LOG"

case "$url" in
  *sha256.sum)
    cat > "$output" <<CHECKSUM_EOF
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa *sush-aarch64-apple-darwin.tar.xz
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb *sush-x86_64-apple-darwin.tar.xz
cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc *sush-aarch64-unknown-linux-gnu.tar.xz
dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd *sush-x86_64-unknown-linux-gnu.tar.xz
CHECKSUM_EOF
    ;;
  *)
    printf 'archive for %s\n' "$url" > "$output"
    ;;
esac
CURL_EOF

cat > "$fake_bin/tar" <<'EOF'
#!/bin/sh
set -eu

dest=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -C)
      dest="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

mkdir -p "$dest"
cat > "$dest/sush" <<'EOF2'
#!/bin/sh
echo sush
EOF2
chmod +x "$dest/sush"
EOF

cat > "$fake_bin/unzip" <<'EOF'
#!/bin/sh
set -eu

dest=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -d)
      dest="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

mkdir -p "$dest"
cat > "$dest/sush" <<'EOF2'
#!/bin/sh
echo sush
EOF2
chmod +x "$dest/sush"
EOF

cat > "$fake_bin/shasum" <<'EOF'
#!/bin/sh
set -eu

if [ "${1:-}" = "-a" ]; then
  shift 2
fi

file="$1"
case "$(basename "$file")" in
  sush-aarch64-apple-darwin.tar.xz)
    printf 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  %s\n' "$file"
    ;;
  sush-x86_64-apple-darwin.tar.xz)
    printf 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  %s\n' "$file"
    ;;
  sush-aarch64-unknown-linux-gnu.tar.xz)
    printf 'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc  %s\n' "$file"
    ;;
  sush-x86_64-unknown-linux-gnu.tar.xz)
    printf 'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd  %s\n' "$file"
    ;;
  *)
    printf '0000000000000000000000000000000000000000000000000000000000000000  %s\n' "$file"
    ;;
esac
EOF

chmod +x "$fake_bin/uname" "$fake_bin/curl" "$fake_bin/tar" "$fake_bin/unzip" "$fake_bin/shasum"

run_install() {
  name="$1"
  os="$2"
  arch="$3"
  version="$4"
  install_dir="$tmp_dir/$name/bin"
  curl_log="$tmp_dir/$name/curl.log"
  mkdir -p "$tmp_dir/$name"
  : > "$curl_log"

  FAKE_UNAME_S="$os" \
  FAKE_UNAME_M="$arch" \
  FAKE_CURL_LOG="$curl_log" \
  PATH="$fake_bin:$PATH" \
  SUSH_INSTALL_DIR="$install_dir" \
  "$install_script" "$version" > "$tmp_dir/$name/out" 2>&1

  test -x "$install_dir/sush"
  printf '%s\n' "$curl_log"
}

run_install_with_env_version() {
  name="$1"
  os="$2"
  arch="$3"
  env_version="$4"
  install_dir="$tmp_dir/$name/bin"
  curl_log="$tmp_dir/$name/curl.log"
  mkdir -p "$tmp_dir/$name"
  : > "$curl_log"

  FAKE_UNAME_S="$os" \
  FAKE_UNAME_M="$arch" \
  FAKE_CURL_LOG="$curl_log" \
  PATH="$fake_bin:$PATH" \
  SUSH_INSTALL_DIR="$install_dir" \
  SUSH_VERSION="$env_version" \
  "$install_script" > "$tmp_dir/$name/out" 2>&1

  test -x "$install_dir/sush"
  printf '%s\n' "$curl_log"
}

curl_log=$(run_install "mac-latest" "Darwin" "arm64" "")
grep -F "https://github.com/0xlic/sush.sh/releases/latest/download/sush-aarch64-apple-darwin.tar.xz" "$curl_log" > /dev/null
grep -F "https://github.com/0xlic/sush.sh/releases/latest/download/sha256.sum" "$curl_log" > /dev/null

curl_log=$(run_install "linux-version" "Linux" "x86_64" "v1.2.0")
grep -F "https://github.com/0xlic/sush.sh/releases/download/v1.2.0/sush-x86_64-unknown-linux-gnu.tar.xz" "$curl_log" > /dev/null

curl_log=$(run_install "linux-env-version" "Linux" "aarch64" "")
grep -F "sush-aarch64-unknown-linux-gnu.tar.xz" "$curl_log" > /dev/null

curl_log=$(run_install_with_env_version "env-version" "Darwin" "x86_64" "1.2.0")
grep -F "https://github.com/0xlic/sush.sh/releases/download/v1.2.0/sush-x86_64-apple-darwin.tar.xz" "$curl_log" > /dev/null

unsupported_dir="$tmp_dir/unsupported"
mkdir -p "$unsupported_dir"
if FAKE_UNAME_S="FreeBSD" \
  FAKE_UNAME_M="x86_64" \
  FAKE_CURL_LOG="$unsupported_dir/curl.log" \
  PATH="$fake_bin:$PATH" \
  SUSH_INSTALL_DIR="$unsupported_dir/bin" \
  "$install_script" > "$unsupported_dir/out" 2>&1; then
  echo "expected unsupported platform failure" >&2
  exit 1
fi
grep -F "unsupported platform: FreeBSD x86_64" "$unsupported_dir/out" > /dev/null
