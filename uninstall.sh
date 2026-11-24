#!/usr/bin/env bash
# Remove niruprodrive and the Proton Drive CLI it installed. The sign-in is
# ended first (the CLI clears its session and caches). With --purge the
# settings and the backup record go too. Nothing in the Drive is touched.
set -euo pipefail
purge=0
for arg in "$@"; do
  case "$arg" in
    --purge) purge=1 ;;
    -h|--help) echo "usage: ./uninstall.sh [--purge]"; exit 0 ;;
    *) echo "unknown option: $arg (try --help)" >&2; exit 2 ;;
  esac
done
BIN="$HOME/.local/bin/niruprodrive"
LIB="$HOME/.local/lib/niruprodrive"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}"
STATE="${XDG_STATE_HOME:-$HOME/.local/state}"
[[ -x $BIN ]] && "$BIN" quit >/dev/null 2>&1 || :
if [[ -x $LIB/proton-drive ]]; then
  "$LIB/proton-drive" auth logout >/dev/null 2>&1 || :
fi
rm -f "$BIN" "$LIB/proton-drive" \
  "$DATA/applications/niruprodrive.desktop" \
  "$DATA/icons/hicolor/scalable/apps/niruprodrive.svg" \
  "$CONFIG/autostart/niruprodrive.desktop"
rmdir "$LIB" 2>/dev/null || :
if [[ $purge == 1 ]]; then
  rm -rf "$CONFIG/niruprodrive" "$STATE/niruprodrive" \
    "$STATE/proton-drive-cli" "$DATA/proton-drive-cli" "${XDG_CACHE_HOME:-$HOME/.cache}/proton-drive-cli"
  echo "niruprodrive removed, with its settings, backup record and the CLI's caches."
else
  echo "niruprodrive removed. Settings stay in $CONFIG/niruprodrive (--purge removes them)."
fi
echo "Your files in Proton Drive are untouched."
