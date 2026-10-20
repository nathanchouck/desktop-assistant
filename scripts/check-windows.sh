#!/usr/bin/env bash
# Lints the Windows build from Linux/macOS (catches errors in #[cfg(windows)]
# code without waiting for CI). Needs: rustup target add x86_64-pc-windows-msvc
#
# Lint never links, so C dependencies (bundled SQLite) don't need a real
# Windows compiler: a stand-in writes empty object files and archives.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
fake="$(mktemp -d)"
trap 'rm -rf -- "${fake:?}"' EXIT

cat > "$fake/cc" <<'SH'
#!/usr/bin/env bash
# Stand-in C compiler: create whatever output file was asked for.
for arg in "$@"; do
  case "$arg" in
    -Fo*|/Fo*) : > "${arg:3}" ;;
    -o) want_next=1; continue ;;
  esac
  if [[ "${want_next:-}" == 1 ]]; then : > "$arg"; want_next=; fi
done
exit 0
SH
cat > "$fake/ar" <<'SH'
#!/usr/bin/env bash
# Stand-in archiver (lib.exe style: -out:<file> or /OUT:<file>).
for arg in "$@"; do
  case "$arg" in
    -out:*|/OUT:*|/out:*) : > "${arg#*:}" ;;
  esac
done
exit 0
SH
chmod +x "$fake/cc" "$fake/ar"

cd "$root/src-tauri"
CC_x86_64_pc_windows_msvc="$fake/cc" \
AR_x86_64_pc_windows_msvc="$fake/ar" \
  cargo clippy --target x86_64-pc-windows-msvc --all-targets "$@" -- -D warnings
