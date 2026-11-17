#!/usr/bin/env bash
# Copyright (c) 2026 DakiaMail
# License: MPL-2.0 | https://github.com/DakiaMail/dakia-web/blob/main/LICENSE
# Source: https://github.com/DakiaMail/dakia-web

umask 022
_CS_DEFAULT_URL="https://raw.githubusercontent.com/DakiaMail/dakia-web/main"
export COMMUNITY_SCRIPTS_CORE_URL="${COMMUNITY_SCRIPTS_CORE_URL:-https://raw.githubusercontent.com/community-scripts/core/2c3410fba1fbb8af16107166f692339fb4c63be9}"
_cs_boot="${COMMUNITY_SCRIPTS_CORE_DIR:-$(dirname "${BASH_SOURCE[0]}")/../../core}/core/build.func"
if [[ -f "$_cs_boot" ]]; then
  # shellcheck source=/dev/null
  source "$_cs_boot" || exit 1
else
  _cs_engine=$(curl -fsSL "${COMMUNITY_SCRIPTS_CORE_URL}/core/build.func") || exit 1
  # shellcheck source=/dev/null
  source /dev/stdin <<<"$_cs_engine" || exit 1
  unset _cs_engine
fi

APP="Dakia"
var_tags="${var_tags:-email}"
var_cpu="${var_cpu:-2}"
var_ram="${var_ram:-4096}"
var_disk="${var_disk:-20}"
var_os="${var_os:-ubuntu}"
var_version="${var_version:-24.04}"
var_arm64="${var_arm64:-yes}"
var_unprivileged="${var_unprivileged:-1}"
var_nesting="${var_nesting:-0}"
export var_dakia_origin="${var_dakia_origin:-}"

header_info "$APP"
variables
color
catch_errors

function update_script() {
  local updater
  updater=$(_cs_fetch_text "scripts/upgrade-native.py") || exit 1
  python3 -c "$updater" || exit 1
}

validate_dakia_origin() {
  python3 - "$var_dakia_origin" <<'PY_ORIGIN'
import ipaddress, re, sys, urllib.parse
try:
    u = urllib.parse.urlsplit(sys.argv[1])
    assert u.scheme == 'https' and u.hostname and u.username is None and u.password is None
    assert not u.path and not u.query and not u.fragment
    assert all(32 < ord(c) < 127 for c in sys.argv[1])
    assert u.port is None or 1 <= u.port <= 65535
    authority = f'[{u.hostname}]' if ':' in u.hostname else u.hostname
    if u.port is not None:
        authority += f':{u.port}'
    assert u.netloc.lower() == authority.lower()
    try:
        ipaddress.ip_address(u.hostname)
    except ValueError:
        assert all(re.fullmatch(r'[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?', label) for label in u.hostname.split('.'))
except (AssertionError, ValueError):
    sys.exit('Use an HTTPS origin without credentials, path, query, or fragment')
PY_ORIGIN
}

if command -v pveversion >/dev/null 2>&1; then
  if [[ -n "${var_ctid:-}" ]]; then
    pvesh get /cluster/resources --type vm --output-format json | python3 -c '
import json, sys
if any(int(vm["vmid"]) == int(sys.argv[1]) for vm in json.load(sys.stdin)):
    sys.exit("Guest ID already exists; no existing guest will be changed")
' "$var_ctid" || exit 1
  fi
  if [[ -z "$var_dakia_origin" ]]; then
    if [[ "${PHS_SILENT:-0}" == 1 ]]; then
      msg_error "Set var_dakia_origin to the intended HTTPS origin for unattended installation."
      exit 1
    fi
    ensure_whiptail
    var_dakia_origin=$(whiptail --title "Dakia" --inputbox "Exact HTTPS origin, for example https://mail.example.com" 10 72 3>&1 1>&2 2>&3) || exit 0
  fi
  validate_dakia_origin || exit 1
fi

start
[[ "$var_os" == ubuntu && "$var_version" == 24.04 ]] || { msg_error "Dakia native releases require Ubuntu 24.04."; exit 1; }
validate_dakia_origin || exit 1
case "$(uname -m)" in x86_64) var_dakia_arch=amd64;; aarch64) var_dakia_arch=arm64;; *) msg_error "Unsupported architecture"; exit 1;; esac
msg_info "Selecting latest stable Dakia release"
_release_json=$(curl -fLsS --retry 3 --connect-timeout 15 --max-time 120 https://api.github.com/repos/DakiaMail/dakia-web/releases/latest) || exit 1
var_dakia_release=$(python3 -c '
import json, re, sys
r = json.load(sys.stdin)
v = r.get("tag_name", "")
if r.get("draft") or r.get("prerelease") or not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", v):
    sys.exit("GitHub did not return a stable semantic release")
required = {f"dakia-{v}-linux-{sys.argv[1]}.tar.gz", "SHA256SUMS"}
available = {a["name"] for a in r.get("assets", []) if a.get("state") == "uploaded" and a.get("size", 0) > 0}
if not required <= available:
    sys.exit("Latest release is missing its native bundle or checksums")
print(v)
' "$var_dakia_arch" <<<"$_release_json") || exit 1
export var_dakia_release var_dakia_arch var_dakia_origin
unset _release_json
msg_ok "Selected Dakia $var_dakia_release"

build_container
description
msg_ok "Dakia setup completed"
echo "Dakia listens on 127.0.0.1:3025 inside the container."
echo "Configure private HTTPS access for $var_dakia_origin, create the owner login, then widen access."
