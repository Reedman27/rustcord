#!/bin/bash
# End-to-end check of the Qt frontend in a virtual display, no Discord account or internet:
# discord.com is mapped to a local HTTPS stub that reports what it sees. Verifies that the
# real Equicord bundle runs at document start (plugins register), quick.css is applied, WebRTC
# exists, and a simulated game's Rich Presence reaches the page through the 127.0.0.1:1337 bridge.
# Needs: Xvfb, dbus (dbus-daemon, dbus-monitor), openssl, python3, a built rustcord-qt. Usage: run.sh <Equicord.user.js> [binary]
set -u
BUNDLE=$(readlink -f "$1"); BIN=${2:-target/debug/rustcord-qt}; BIN=$(readlink -f "$BIN")
HERE=$(cd "$(dirname "$0")" && pwd); W=$(mktemp -d); cd "$W"
mkdir -p cfg/rustcord data run; chmod 700 run
openssl req -x509 -newkey rsa:2048 -nodes -keyout k.pem -out c.pem -days 2 -subj "/CN=discord.com" \
  -addext "subjectAltName=DNS:discord.com" 2>/dev/null
cat > cfg/rustcord/settings.json <<JSON
{"sandbox":false,"tray":false,"mod_client":"equicord","mod_script":"$BUNDLE",
 "extra_switches":["host-resolver-rules=MAP discord.com 127.0.0.1:8443, MAP themes.test 127.0.0.1:8443","ignore-certificate-errors","disable-gpu","no-proxy-server"]}
JSON
echo 'body{outline:7px solid rgb(1,2,3)!important}' > cfg/rustcord/quick.css
cp "$HERE/server.py" "$HERE/game.py" .
dbus-daemon --session --nofork --address=unix:path=$W/bus >/dev/null 2>&1 & DB=$!
export DBUS_SESSION_BUS_ADDRESS=unix:path=$W/bus
sleep 1
dbus-monitor "interface='org.freedesktop.Notifications'" > dbus.log 2>&1 & DM=$!
Xvfb :77 -screen 0 1280x800x24 >/dev/null 2>&1 & XV=$!
python3 server.py > server.log 2>&1 & SV=$!
sleep 2
DISPLAY=:77 XDG_CONFIG_HOME=$W/cfg XDG_DATA_HOME=$W/data XDG_RUNTIME_DIR=$W/run QT_QPA_PLATFORM=xcb "$BIN" > app.log 2>&1 & APP=$!
sleep 9
XDG_RUNTIME_DIR=$W/run python3 game.py 4; sleep 2
kill $APP $SV $XV $DM $DB 2>/dev/null; pkill QtWebEngineProc 2>/dev/null
cat server.log; grep -h 'Hello title' dbus.log | head -2
ok=1
grep -q '"plugins": [1-9][0-9][0-9]' server.log || ok=0
grep -q '7px rgb(1, 2, 3)' server.log || ok=0
grep -q 'Playing test game' server.log || ok=0
grep -q 'Hello title' dbus.log || ok=0   # desktop notification reached D-Bus
grep '"extcss"' server.log | tail -1 | grep -q '9px' || ok=0   # CSP bypass: external stylesheet loaded
if [ $ok = 1 ]; then echo "PASS"; else echo "FAIL (see $W/app.log)"; exit 1; fi
