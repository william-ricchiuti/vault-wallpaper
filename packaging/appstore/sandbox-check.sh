#!/usr/bin/env bash
# Walks through the sandboxed (App Store) build on this Mac and checks each feature works
# inside the App Sandbox. Some steps need a person: the sandbox's open panel only accepts
# real clicks, and the menu-bar menu can't be scripted.
#
# Usage: VW_APPSTORE_ADHOC=1 packaging/appstore/build.sh && packaging/appstore/sandbox-check.sh
#
# It reads, never deletes, the container at
# ~/Library/Containers/com.williamricchiuti.vaultwallpaper. For a true first run, quit the app
# and move that folder to the Trash first.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
APP="${1:-$ROOT/dist/appstore/Vault Wallpaper.app}"
BIN="$APP/Contents/MacOS/vault-wallpaper"
SUPPORT="$HOME/Library/Containers/com.williamricchiuti.vaultwallpaper/Data/Library/Application Support/Vault Wallpaper"
LOGS="$(mktemp -d -t vw-sandbox-check)"
ERR="$LOGS/app.err"
SBX="$LOGS/sandbox.log"
pass=0; fail=0
ok()   { echo "  PASS $*"; pass=$((pass + 1)); }
bad()  { echo "  FAIL $*"; fail=$((fail + 1)); }
ask()  { read -r -p "  -> $* (Enter when done) " _; }
yes()  { local a; read -r -p "  -> $* [y/n] " a; [[ "$a" == y* ]]; }
quit() { pkill -f "$BIN" 2>/dev/null; sleep 1; }
launch() { : >"$ERR"; open --stderr "$ERR" "$APP"; sleep "${1:-4}"; }

[[ -x "$BIN" ]] || { echo "no app at $APP; run VW_APPSTORE_ADHOC=1 packaging/appstore/build.sh" >&2; exit 1; }
echo "== signature"
ENT="$(codesign -d --entitlements :- "$APP" 2>/dev/null)"
grep -q com.apple.security.app-sandbox <<<"$ENT" && ok "sandbox entitlement present" || bad "not sandboxed"

/usr/bin/log stream --style compact --predicate 'sender == "Sandbox" AND eventMessage CONTAINS "vault-wallpaper"' >"$SBX" 2>&1 &
LOGPID=$!
trap 'kill $LOGPID 2>/dev/null' EXIT
quit

echo "== 1. first run: vault picker and bookmark"
launch
if [[ -f "$SUPPORT/vault.bookmark" ]]; then
  echo "  (a bookmark exists already; use Choose Vault… from the menu to pick again)"
  ask "Menu bar icon > Choose Vault…, pick your vault, click Open"
else
  ask "In the \"Choose your Obsidian vault\" panel, pick your vault and click Open"
fi
sleep 3
[[ -f "$SUPPORT/vault.bookmark" ]] && ok "vault.bookmark saved in the container" || bad "no vault.bookmark in $SUPPORT"
grep -q '"vaultPath"' "$SUPPORT/config.json" 2>/dev/null && ok "config.json in the container has vaultPath" || bad "no vaultPath in $SUPPORT/config.json"
grep -E 'scanned [1-9][0-9]* notes' "$ERR" >/dev/null && ok "$(grep -oE 'scanned [0-9]+ notes' "$ERR" | tail -1)" || bad "vault not scanned (see $ERR)"

echo "== 2. relaunch: the bookmark reopens the vault with no panel"
quit; launch 6
grep -q 'vault access:' "$ERR" && bad "$(grep 'vault access:' "$ERR" | tail -1)" || ok "no access error"
grep -E 'scanned [1-9][0-9]* notes' "$ERR" >/dev/null && ok "after relaunch: $(grep -oE 'scanned [0-9]+ notes' "$ERR" | tail -1)" || bad "not scanned after relaunch"

echo "== 3. watching (FSEvents under the bookmark)"
ask "Edit and save any note in Obsidian"
sleep 2
grep -q 'vault changed' "$ERR" && ok "the watcher saw the edit" || bad "no 'vault changed' line"
if [[ -n "$(find "$(sed -n 's/.*"vaultPath": *"\([^"]*\)".*/\1/p' "$SUPPORT/config.json")/.obsidian/plugins/vault-wallpaper" -name state.json 2>/dev/null)" ]]; then
  grep -q 'obsidian:' "$ERR" && ok "the Obsidian plugin's state.json is read" || bad "plugin installed but no 'obsidian:' line"
else
  echo "  (Obsidian plugin not installed in this vault; state.json not checked)"
fi

echo "== 4. settings window (WKWebView, vw:// scheme)"
ask "Menu bar icon > Settings…, click a different theme tile, close the window"
grep -q 'config reloaded' "$ERR" && ok "a settings change reached the config" || bad "no 'config reloaded' line"
yes "Did the settings page show (not blank) and the wallpaper change?" && ok "settings page shown" || bad "settings page"

echo "== 5. Export PNG to ~/Pictures/Vault Wallpaper"
before="$(ls "$HOME/Pictures/Vault Wallpaper" 2>/dev/null | wc -l)"
ask "Menu bar icon > Export PNG"
sleep 2
after="$(ls "$HOME/Pictures/Vault Wallpaper" 2>/dev/null | wc -l)"
((after > before)) && ok "new file: $(ls -t "$HOME/Pictures/Vault Wallpaper" | head -1)" || bad "no new PNG ($(grep 'export failed' "$ERR" | tail -1))"

echo "== 6. Import Web Version Settings… (open panel)"
if yes "Do you have a web-version config.json to try? (it replaces this build's settings)"; then
  ask "Menu bar icon > Import Web Version Settings…, pick it, confirm; grant presets/vault if asked"
  grep -q 'config reloaded' "$ERR" && ok "imported config loaded" || bad "no reload after import"
fi

echo "== 7. Start at Login (SMAppService.mainApp)"
ask "Menu bar icon > Start at Login (tick it)"
yes "Is it ticked, and listed in System Settings > General > Login Items?" && ok "login item on" || bad "login item"
ask "Menu bar icon > Start at Login again (untick it)"
grep -q 'start at login:' "$ERR" && bad "$(grep 'start at login:' "$ERR" | tail -1)" || ok "no Start at Login error"

echo "== 8. Open Config File"
ask "Menu bar icon > Open Config File"
yes "Did config.json open in an editor?" && ok "config opened" || bad "open config"

quit
echo "== sandbox violations from vault-wallpaper"
grep -v '^Filtering\|^Timestamp' "$SBX" | sed 's/.*Sandbox: //' | sort | uniq -c | sort -rn | head -20
echo "== $pass passed, $fail failed. Logs: $LOGS"
