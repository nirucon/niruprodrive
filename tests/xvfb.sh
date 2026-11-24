#!/bin/sh
# End-to-end in a private Xvfb with a stand-in for the Proton Drive CLI
# (tests/fake-proton-drive, "the Drive" a folder): sign-in, backup, a
# changed file, a deleted one, files sent from the command line, a link on
# the clipboard, the badges' answers, and that nothing that removes or
# replaces was ever asked of the CLI.
# Usage: tests/xvfb.sh PATH/TO/niruprodrive [SHOTS-DIR]
set -eu
BIN=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
HERE=$(cd "$(dirname "$0")" && pwd)
SHOTS=${2:-}
SB=$(mktemp -d)
PIDS=""
cleanup() {
  for p in $PIDS; do kill "$p" 2>/dev/null || true; done
  if [ -n "${KEEP:-}" ]; then echo "sandbox $SB"; else rm -rf "$SB"; fi
}
trap cleanup EXIT INT TERM
export HOME=$SB/home XDG_CONFIG_HOME=$SB/home/.config XDG_STATE_HOME=$SB/home/.local/state
export XDG_RUNTIME_DIR=$SB/run FAKE_DRIVE=$SB/fake NIRUPRODRIVE_CLI=$HERE/fake-proton-drive
mkdir -p "$HOME/Documents/work" "$HOME/Pictures" "$XDG_RUNTIME_DIR" "$FAKE_DRIVE" "$SB/bin"
chmod 700 "$XDG_RUNTIME_DIR"
# xdg-open and notify-send write down what they were asked.
printf '#!/bin/sh\necho "$@" >> %s/opened\n' "$SB" > "$SB/bin/xdg-open"
printf '#!/bin/sh\necho "$@" >> %s/notified\n' "$SB" > "$SB/bin/notify-send"
chmod +x "$SB/bin/xdg-open" "$SB/bin/notify-send"
export PATH="$SB/bin:$PATH"
echo "first" > "$HOME/Documents/notes.txt"
echo "a" > "$HOME/Documents/work/a.txt"
echo "b" > "$HOME/Documents/work/b.txt"
mkdir -p "$HOME/Documents/code/target"
echo "fn main() {}" > "$HOME/Documents/code/main.rs"
echo "build junk" > "$HOME/Documents/code/target/junk.o"
head -c 2000 /dev/urandom > "$HOME/Pictures/photo.jpg"
mkdir -p "$XDG_CONFIG_HOME/niruprodrive"
cat > "$XDG_CONFIG_HOME/niruprodrive/config.toml" <<EOF
[backup]
folders = ["~/Documents", "~/Pictures"]
every = "manual"
[drive]
computer = "testhost"
EOF

D=:$((50 + $$ % 20))
Xvfb "$D" -screen 0 1280x800x24 -nolisten tcp >/dev/null 2>&1 &
PIDS="$PIDS $!"
export DISPLAY=$D
sleep 1
fail() { echo "FAIL: $*" >&2; [ -f "$FAKE_DRIVE/calls.log" ] && tail -20 "$FAKE_DRIVE/calls.log" >&2; exit 1; }
shot() { [ -n "$SHOTS" ] && ffmpeg -loglevel error -y -f x11grab -video_size 1280x800 -i "$D" -frames:v 1 "$SHOTS/$1.png" || true; }
state() { "$BIN" status 2>/dev/null; }
idle() { s=$(state); echo "$s" | grep -q '"running": 0' && echo "$s" | grep -q '"queued": 0'; }
wait_for() { # SECONDS COMMAND...
  n=$(( $1 * 10 )); shift
  while [ $n -gt 0 ]; do "$@" && return 0; sleep 0.1; n=$((n - 1)); done
  return 1
}

"$BIN" >"$SB/out.log" 2>&1 &
APP=$!
PIDS="$PIDS $APP"
wait_for 5 test -S "$XDG_RUNTIME_DIR/niruprodrive.sock" || fail "no socket"
wait_for 5 sh -c "\"$BIN\" status | grep -q '\"signed\": \"Out\"'" || fail "not signed out at start: $(state)"
rss=$(awk '/VmRSS/ {print $2}' /proc/$APP/status)
[ "$rss" -lt 40000 ] || fail "memory $rss kB"
echo "started, ${rss} kB, signed out"
# A second start only shows the window of the first.
"$BIN" || fail "second start"
[ "$(pgrep -c -f "^$BIN" || true)" -ge 1 ]
W=$(xdotool search --sync --onlyvisible --class niruprodrive | head -1)
xdotool windowmove "$W" 0 0; sleep 0.3
shot signin

# Sign in: Enter, the browser gets Proton's page, the sign-in completes.
xdotool windowfocus --sync "$W" key Return
wait_for 5 sh -c "grep -qs fake-sign-in \"$SB/opened\"" || fail "the sign-in page was not opened"
shot waiting
touch "$FAKE_DRIVE/approve"
wait_for 5 sh -c "\"$BIN\" status | grep -q '\"signed\": \"In\"'" || fail "not signed in: $(state)"
echo "sign-in ok"

# The first backup: new folders whole, code/ file by file without target/.
"$BIN" backup || fail "backup did not start"
wait_for 20 sh -c "\"$BIN\" status | grep -q '\"last_ok\": [1-9]'" || fail "backup: $(state)"
R=$FAKE_DRIVE/drive/my-files/NIRUX/testhost
[ -f "$R/Documents/notes.txt" ] && [ -f "$R/Documents/work/b.txt" ] && [ -f "$R/Pictures/photo.jpg" ] \
  || fail "backup is missing files: $(cd "$FAKE_DRIVE/drive" && find . | head -30)"
[ -f "$R/Documents/code/main.rs" ] || fail "code/main.rs missing"
[ ! -e "$R/Documents/code/target" ] || fail "excluded target/ was sent"
grep -q "upload -f create-new-revision -d merge .*/Documents/work " "$FAKE_DRIVE/calls.log" \
  || fail "work/ was not sent as one folder"
echo "first backup ok"

# Unchanged: nothing is sent. One file changed: only it, as a new revision.
: > "$FAKE_DRIVE/calls.log"
"$BIN" backup; sleep 0.3
wait_for 10 idle
! grep -q "filesystem upload" "$FAKE_DRIVE/calls.log" || fail "an unchanged backup uploaded: $(cat "$FAKE_DRIVE/calls.log")"
sleep 1.1
echo "second" > "$HOME/Documents/work/a.txt"
rm "$HOME/Documents/notes.txt"
: > "$FAKE_DRIVE/calls.log"
"$BIN" backup; sleep 0.3
wait_for 10 idle
[ "$(grep -c "filesystem upload" "$FAKE_DRIVE/calls.log")" = 1 ] || fail "not one upload: $(cat "$FAKE_DRIVE/calls.log")"
grep -q "upload .*work/a.txt /my-files/NIRUX/testhost/Documents/work --json" "$FAKE_DRIVE/calls.log" || fail "wrong upload: $(cat "$FAKE_DRIVE/calls.log")"
[ "$(cat "$R/Documents/work/a.txt")" = second ] || fail "the change did not arrive"
ls "$FAKE_DRIVE/revisions" | grep -q "work_a.txt" || fail "the old version was not kept as a revision"
[ -f "$R/Documents/notes.txt" ] || fail "a file deleted here was removed in the Drive"
echo "changes ok (one upload, a revision, nothing removed)"

# Files sent from the command line (NIRUFM does the same over the socket).
echo "report" > "$SB/report.txt"
"$BIN" upload "$SB/report.txt" || fail "upload"
wait_for 10 test -f "$FAKE_DRIVE/drive/my-files/NIRUX/Uploads/report.txt" || fail "upload did not arrive"
wait_for 3 sh -c "grep -qs \"Sent to Proton Drive\" \"$SB/notified\"" || fail "no notice for the upload"

# A link: on the clipboard, with a notice. The backed-up copy is linked.
"$BIN" link "$HOME/Pictures/photo.jpg" 7 || fail "link"
wait_for 10 sh -c "xclip -o -selection clipboard 2>/dev/null | grep -q drive.proton.me/urls/FAKE" \
  || fail "no link on the clipboard: $(cat "$SB/notified")"
grep -q '"expires": "20' "$FAKE_DRIVE/links.json" || fail "the link has no end date"
grep -q "Pictures/photo.jpg" "$FAKE_DRIVE/links.json" || fail "the backed-up copy was not linked"
echo "upload and link ok"

# The badges' answers.
python3 - "$XDG_RUNTIME_DIR/niruprodrive.sock" "$HOME" <<'PY' || fail "status answers"
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1])
h = sys.argv[2]
s.sendall((json.dumps({"op": "status", "paths": [h + "/Documents/work/a.txt", h + "/Documents/code/target/junk.o", "/etc/hostname"]}) + "\n").encode())
r = json.loads(s.makefile().readline())
assert r["marks"] == ["done", "none", "none"], r
PY
echo "new" > "$HOME/Documents/fresh.txt"
python3 - "$XDG_RUNTIME_DIR/niruprodrive.sock" "$HOME" <<'PY' || fail "pending mark"
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1])
s.sendall((json.dumps({"op": "status", "paths": [sys.argv[2] + "/Documents/fresh.txt"]}) + "\n").encode())
assert json.loads(s.makefile().readline())["marks"] == ["pending"]
PY
echo "badges ok"

# The window: the Drive lists this computer's folder; files dropped… are
# covered by NIRUFM's own test. Screenshots of the three tabs.
xdotool windowfocus --sync "$W" key 1; sleep 0.8
shot drive
xdotool key 2; sleep 0.3
shot transfers
xdotool key 3; sleep 0.3
shot backup
xdotool key 1; sleep 0.2
grep -q "filesystem list /my-files/NIRUX" "$FAKE_DRIVE/calls.log" || fail "the Drive tab did not list"

# A file that fails is reported and sent again next time.
echo x > "$HOME/Documents/FAIL-ME"
"$BIN" backup; sleep 0.3
wait_for 10 idle
"$BIN" status | grep -q 'not sent: FAIL-ME' || fail "a failure was not reported: $(state)"
rm "$HOME/Documents/FAIL-ME"

# NIRUFM beside it (NIRUFM=path): badges on the backed-up files, and its
# menu sends a file.
if [ -n "${NIRUFM:-}" ]; then
  "$NIRUFM" "$HOME/Documents" 2>/dev/null &
  FM=$!
  PIDS="$PIDS $FM"
  WF=$(xdotool search --sync --onlyvisible --pid "$FM" | head -1)
  xdotool windowmove "$WF" 0 0 windowsize "$WF" 1000 600; xdotool windowraise "$WF"; sleep 1
  sleep 0.5
  shot nirufm-badges
  # Right-click the folder "work", then "Send to Proton Drive" (the eighth
  # item of a folder's menu): it arrives in Uploads.
  xdotool mousemove 376 100 click 3; sleep 0.5
  shot nirufm-menu
  xdotool mousemove 430 $((100 + 175)) click 1; sleep 0.3
  wait_for 10 test -f "$FAKE_DRIVE/drive/my-files/NIRUX/Uploads/work/b.txt" || fail "NIRUFM's menu did not send"
  echo "NIRUFM ok"
fi

[ ! -e "$FAKE_DRIVE/FORBIDDEN" ] || fail "forbidden commands: $(cat "$FAKE_DRIVE/FORBIDDEN")"
grep -Eq "^(filesystem (trash|delete|move|rename|restore|empty-trash)|sharing remove )" "$FAKE_DRIVE/calls.log" && fail "removal asked for"
"$BIN" quit
wait_for 5 sh -c "! kill -0 $APP 2>/dev/null" || fail "did not quit"
[ ! -S "$XDG_RUNTIME_DIR/niruprodrive.sock" ] || fail "socket left behind"
echo "PASS"
