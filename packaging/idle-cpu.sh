#!/usr/bin/env bash
# Idle CPU with one SQLite connection open (spec §7: < 1%). Needs Xvfb, pidstat (sysstat), xwd,
# python3 and libXtst. Usage: packaging/idle-cpu.sh [target/release/crustdb] [click_x click_y]
# The click (default 58 174: the "Connect" button of the only saved connection on a 1280x720 screen)
# opens the connection (NOCLICK=1 skips it: home-screen control); set SHOT=out.png to keep a screenshot taken after the click and check it.
set -euo pipefail
bin=${1:-target/release/crustdb}
cx=${2:-58}
cy=${3:-174}
tmp=$(mktemp -d)
mkdir -p "$tmp/cfg/crustdb"
: >"$tmp/t.db" # an empty file is a valid SQLite database
app=
cat >"$tmp/cfg/crustdb/connections.json" <<JSON
{"version":1,"connections":[{"id":"00000000-0000-0000-0000-000000000001","name":"idle","driver":"sqlite","host":"","port":0,"user":"","database":"$tmp/t.db","save_password":false,"tls":{"mode":"disable"},"ssh":null,"page_size":300,"editor_row_limit":1000,"safety_threshold":1000}]}
JSON
Xvfb :97 -screen 0 1280x720x24 >/dev/null 2>&1 &
xpid=$!
cleanup() { kill ${app:+"$app"} "$xpid" 2>/dev/null || true; rm -rf "$tmp"; }
trap cleanup EXIT
sleep 1
DISPLAY=:97 XDG_CONFIG_HOME="$tmp/cfg" XDG_DATA_HOME="$tmp/data" XDG_STATE_HOME="$tmp/state" \
  CRUSTDB_OFFLINE=1 "$bin" >/dev/null 2>&1 &
app=$!
sleep 4
[ -n "${NOCLICK:-}" ] || DISPLAY=:97 python3 -I - "$cx" "$cy" <<'PY'
import ctypes, sys, time
x11 = ctypes.CDLL("libX11.so.6"); xt = ctypes.CDLL("libXtst.so.6")
x11.XOpenDisplay.restype = ctypes.c_void_p
d = x11.XOpenDisplay(None)
xt.XTestFakeMotionEvent(ctypes.c_void_p(d), 0, int(sys.argv[1]), int(sys.argv[2]), 0)
for down in (1, 0):
    xt.XTestFakeButtonEvent(ctypes.c_void_p(d), 1, down, 0)
    x11.XFlush(ctypes.c_void_p(d)); time.sleep(0.08)
PY
sleep 3
if [ -n "${SHOT:-}" ]; then DISPLAY=:97 import -window root "$SHOT"; fi
pidstat -p "$app" 1 30 | tail -n 3
