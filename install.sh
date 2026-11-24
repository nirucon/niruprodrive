#!/usr/bin/env bash
# niruprodrive installer: the program, Proton's Proton Drive CLI that it
# works through, a menu entry, autostart, and the service in the running
# session. No root, no system files.
#
#   ./install.sh             the prebuilt release binary, checked with SHA-256
#   ./install.sh --build     build this checkout with cargo
#   ./install.sh --no-start  do not start the service now
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"
BIN="$HOME/.local/bin"
LIB="$HOME/.local/lib/niruprodrive"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
APPS="$DATA/applications"
ICONS="$DATA/icons/hicolor/scalable/apps"
AUTOSTART="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"
STATE="${XDG_STATE_HOME:-$HOME/.local/state}/niruprodrive"

# Proton's CLI, pinned: the version and the SHA-512 Proton publishes for it
# (https://proton.me/download/drive/cli/version.json). A new version is
# taken in a niruprodrive release, after testing.
CLI_VERSION="0.9.0"
CLI_SHA512_X64="3533025ba69ae112b64e3e01fbcc1ad0688136a4043f6cf6a72886967d85fdcd9ec235479c2e113171614be5225bfba93427509a05ca5aa6071d924fa7e91ca8"
CLI_SHA512_X64_MUSL="8f99c3bc478ebc2948ca69c8576a48d83b3290d807ca61fac9b0f5d0a91c0d418e595232d67e2ae0173d2188963771afd4292aefafab3f64e808a61c4399d6ed"

MODE=auto
START=1
for arg in "$@"; do
  case "$arg" in
    --build) MODE=build ;;
    --prebuilt) MODE=prebuilt ;;
    --no-start) START=0 ;;
    -y|--yes) ;;
    -h|--help) echo "usage: ./install.sh [--build|--prebuilt] [--no-start]"; exit 0 ;;
    *) echo "unknown option: $arg (try --help)" >&2; exit 2 ;;
  esac
done
[[ $(id -u) != 0 ]] || { echo "Run ./install.sh as your own user, not root." >&2; exit 1; }
[[ $(uname -m) == x86_64 ]] || { echo "niruprodrive is made for x86_64 Linux." >&2; exit 1; }

VERSION="${NIRUPRODRIVE_VERSION:-}"
[[ -z $VERSION && -f Cargo.toml ]] && VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
[[ -n $VERSION ]] || { echo "Cannot tell the version (set NIRUPRODRIVE_VERSION)." >&2; exit 1; }
printf '\033[1mniruprodrive %s\033[0m: Proton Drive backup and sharing for NIRUX\n' "$VERSION"
echo "A third-party application, not made or supported by Proton."

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
fetch() { # URL FILE
  if command -v curl >/dev/null; then
    curl -fsSL --retry 3 --connect-timeout 20 -o "$2" "$1"
  elif command -v wget >/dev/null; then
    wget -qO "$2" "$1"
  else
    echo "curl or wget is needed" >&2; return 1
  fi
}
verify256() {
  local file=$1 sums=$2 name expected actual
  name=$(basename "$file")
  expected=$(awk -v n="$name" '$2 == n || $2 == "*"n {print $1}' "$sums" | head -1)
  [[ -n $expected ]] || { echo "SHA256SUMS has no line for $name" >&2; exit 1; }
  actual=$(sha256sum "$file" | awk '{print $1}')
  [[ $actual == "$expected" ]] || { echo "Checksum mismatch for $name: not installed." >&2; exit 1; }
}

# --- niruprodrive itself
if [[ $MODE == build ]]; then
  command -v cargo >/dev/null || { echo "Rust/cargo is needed for --build" >&2; exit 1; }
  cargo build --release --locked
  SRC=target/release/niruprodrive
elif [[ $MODE == prebuilt || ( $MODE == auto && -x ./niruprodrive ) ]]; then
  SRC=./niruprodrive
  [[ -f SHA256SUMS ]] && verify256 "$SRC" SHA256SUMS
else
  URL="${NIRUPRODRIVE_RELEASE_URL:-https://github.com/nirucon/niruprodrive/releases/download/v${VERSION}}"
  echo "Downloading niruprodrive…"
  fetch "$URL/niruprodrive" "$TMP/niruprodrive"
  fetch "$URL/SHA256SUMS" "$TMP/SHA256SUMS"
  verify256 "$TMP/niruprodrive" "$TMP/SHA256SUMS"
  SRC="$TMP/niruprodrive"
fi

# --- Proton's CLI (glibc build, or the musl one on a musl system)
if ldd --version 2>&1 | grep -qi musl; then
  CLI_PLATFORM=linux-x64-musl; CLI_SHA512=$CLI_SHA512_X64_MUSL
else
  CLI_PLATFORM=linux-x64; CLI_SHA512=$CLI_SHA512_X64
fi
have_cli() {
  [[ -x $LIB/proton-drive ]] && [[ $(sha512sum "$LIB/proton-drive" | awk '{print $1}') == "$CLI_SHA512" ]]
}
if have_cli; then
  echo "Proton Drive CLI $CLI_VERSION is in place."
else
  echo "Downloading Proton's Proton Drive CLI $CLI_VERSION (about 100 MB) from proton.me…"
  fetch "https://proton.me/download/drive/cli/$CLI_VERSION/$CLI_PLATFORM/proton-drive" "$TMP/proton-drive"
  actual=$(sha512sum "$TMP/proton-drive" | awk '{print $1}')
  [[ $actual == "$CLI_SHA512" ]] || { echo "The Proton Drive CLI failed its SHA-512 check: not installed." >&2; exit 1; }
  echo "SHA-512 checked."
  mkdir -p "$LIB"
  install -m 0755 "$TMP/proton-drive" "$LIB/.proton-drive.new.$$"
  mv -f "$LIB/.proton-drive.new.$$" "$LIB/proton-drive"
fi
grep -qw avx2 /proc/cpuinfo 2>/dev/null ||
  echo "Note: this CPU has no AVX2; Proton's CLI may not run on it."

# --- the program, its menu entry, icon and autostart
mkdir -p "$BIN" "$APPS" "$ICONS" "$AUTOSTART" "$STATE"
install -m 0755 "$SRC" "$BIN/.niruprodrive.new.$$"
mv -f "$BIN/.niruprodrive.new.$$" "$BIN/niruprodrive"
install -m 0644 "$ROOT/assets/niruprodrive.svg" "$ICONS/niruprodrive.svg"
sed "s|^Exec=niruprodrive|Exec=$BIN/niruprodrive|" "$ROOT/assets/niruprodrive.desktop" > "$APPS/niruprodrive.desktop"
sed "s|^Exec=niruprodrive|Exec=$BIN/niruprodrive|" "$ROOT/assets/niruprodrive-autostart.desktop" > "$AUTOSTART/niruprodrive.desktop"
chmod 0644 "$APPS/niruprodrive.desktop" "$AUTOSTART/niruprodrive.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" >/dev/null 2>&1 || :
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$DATA/icons/hicolor" >/dev/null 2>&1 || :
"$BIN/niruprodrive" --version

# --- start (or restart) the service in the running X session, also when
# this runs over SSH or from `nirux extras install`.
session_env() {
  [ -n "${DISPLAY:-}" ] && return 0
  for name in niruwm Xorg X; do
    for pid in $(pgrep -u "$(id -u)" -x "$name" 2>/dev/null); do
      [ -r "/proc/$pid/environ" ] || continue
      d=$(tr '\0' '\n' < "/proc/$pid/environ" | sed -n 's/^DISPLAY=//p' | head -n1)
      [ -n "$d" ] || continue
      export DISPLAY="$d"
      for v in XAUTHORITY XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS; do
        val=$(tr '\0' '\n' < "/proc/$pid/environ" | sed -n "s/^$v=//p" | head -n1)
        [ -n "$val" ] && export "$v=$val"
      done
      return 0
    done
  done
  return 1
}
if [[ $START == 1 ]]; then
  cd "$HOME"
  "$BIN/niruprodrive" quit >/dev/null 2>&1 || :
  if session_env; then
    for _ in 1 2 3 4 5 6 7 8 9 10; do
      "$BIN/niruprodrive" status >/dev/null 2>&1 || break
      sleep 0.3
    done
    setsid "$BIN/niruprodrive" service </dev/null >>"$STATE/service.log" 2>&1 &
    sleep 0.8
    if "$BIN/niruprodrive" status >/dev/null 2>&1; then
      echo "The service runs: the cloud in the bar opens niruprodrive."
    else
      echo "The service starts with your next session (or run: niruprodrive)."
    fi
  else
    echo "The service starts with your next session (or run: niruprodrive)."
  fi
fi
case ":$PATH:" in *":$BIN:"*) ;; *) echo "Add $BIN to PATH to run niruprodrive." ;; esac
echo "Done. Open niruprodrive (the cloud in the bar), sign in, and choose folders on the Backup tab."
