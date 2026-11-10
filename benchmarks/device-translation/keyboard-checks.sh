#!/bin/sh
set -eu
session=${DAKIA_TRANSLATION_BROWSER_SESSION:-dakia-translation}
base=${DAKIA_TRANSLATION_URL:-http://127.0.0.1:4178}
chrome=${DAKIA_CHROME_PATH:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}
ab() { agent-browser --session "$session" "$@"; }
assert_focus() {
  ab eval "(() => { if (document.activeElement.id !== '$1') throw new Error('Unexpected keyboard focus'); if (getComputedStyle(document.activeElement).outlineStyle === 'none') throw new Error('No visible focus outline'); return document.activeElement.id; })()"
}
ab --executable-path "$chrome" open "$base"
ab wait --fn 'window.prototypeReady === true'
ab press Tab
assert_focus run
ab press Tab
assert_focus offline
ab press Tab
assert_focus export
ab press Shift+Tab
assert_focus offline
ab open "$base/native.html"
ab wait --fn 'window.nativeReady === true'
ab press Tab
assert_focus source
ab press j
ab eval '(() => { if(document.querySelector("#source").value !== "ja") throw new Error("Keyboard selection failed"); return "Japanese selected by keyboard"; })()'
ab press Tab
assert_focus run
ab press Shift+Tab
assert_focus source
