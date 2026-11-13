#!/bin/sh
# Type-checks the Windows build of the prototype from a Mac:
#   proto/check-windows.sh
# Needs the x86_64-pc-windows-msvc Rust target (rustup target add). tauri-build
# compiles a Windows resource file, which needs llvm-rc; `cargo check` never
# links it, so a stub that writes an empty .res stands in. This proves the
# Windows code compiles, not that it runs: that is the Windows run sheet.
set -eu

cd "$(dirname "$0")"
stub_dir=$(mktemp -d)
trap 'rm -rf "$stub_dir"' EXIT
cat >"$stub_dir/llvm-rc" <<'EOF'
#!/bin/sh
# Answers embed-resource's probe like llvm-rc, then writes an empty .res.
for a in "$@"; do
  if [ "$a" = "/?" ]; then
    echo "OVERVIEW: Resource Converter (stub) no-preprocess"
    exit 0
  fi
done
out=""
while [ $# -gt 0 ]; do
  case "$1" in
    /fo | -fo | /FO | -FO)
      out=$2
      shift
      ;;
  esac
  shift
done
if [ -n "$out" ]; then
  : >"$out"
fi
EOF
chmod +x "$stub_dir/llvm-rc"
RC="$stub_dir/llvm-rc" CARGO_TARGET_DIR=target/windows-check \
  cargo clippy --locked --target x86_64-pc-windows-msvc --all-targets -- -D warnings
