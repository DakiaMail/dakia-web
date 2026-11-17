#!/usr/bin/env bash
# Copyright (c) 2026 DakiaMail
# License: MPL-2.0 | https://github.com/DakiaMail/dakia-web/blob/main/LICENSE
# Source: https://github.com/DakiaMail/dakia-web

# shellcheck source=/dev/null
source /dev/stdin <<<"${FUNCTIONS_FILE_PATH:?Run through ct/dakia.sh}"
color
verb_ip6
catch_errors
# shellcheck source=/dev/null
source /etc/os-release
[[ "$ID" == ubuntu && "$VERSION_ID" == 24.04 ]] || { msg_error "Dakia requires Ubuntu 24.04."; exit 1; }
version=${var_dakia_release:?Missing selected release}
arch=${var_dakia_arch:?Missing selected architecture}
origin=${var_dakia_origin:?Missing HTTPS origin}
[[ "$(dpkg --print-architecture)" == "$arch" ]] || { msg_error "Native bundle architecture does not match the guest."; exit 1; }
[[ ! -e /opt/dakia && ! -e /var/lib/dakia ]] || { msg_error "Dakia already exists; refusing to replace its data or key."; exit 1; }

setting_up_container
network_check
update_os

msg_info "Installing runtime libraries"
$STD apt-get install -y --no-install-recommends ca-certificates curl libssl3t64 libstdc++6 libatomic1
msg_ok "Installed runtime libraries"
msg_info "Downloading and verifying Dakia $version"
useradd --system --home-dir /var/lib/dakia --shell /usr/sbin/nologin dakia
install -d -o dakia -g dakia -m 700 /var/lib/dakia/download /var/lib/dakia/data /var/lib/dakia-secrets
runuser -u dakia -- sh -s -- "$version" "$arch" <<'DOWNLOAD'
set -eu
umask 077
cd /var/lib/dakia/download
archive="dakia-$1-linux-$2.tar.gz"
base="https://github.com/DakiaMail/dakia-web/releases/download/$1"
curl -fLsS --retry 3 --connect-timeout 15 --max-time 900 "$base/$archive" -o "$archive"
curl -fLsS --retry 3 --connect-timeout 15 --max-time 120 "$base/SHA256SUMS" -o SHA256SUMS
awk -v archive="$archive" '$2 == archive' SHA256SUMS > "$archive.sha256"
test "$(wc -l < "$archive.sha256")" -eq 1
sha256sum --check --strict "$archive.sha256"
DOWNLOAD
msg_ok "Verified release checksum"
msg_info "Installing native runtime and service"
release="/opt/dakia/releases/$version"
install -d -m 755 "$release"
tar -xzf "/var/lib/dakia/download/dakia-$version-linux-$arch.tar.gz" -C "$release"
test "$(cat "$release/VERSION")" = "$version"
chown -R root:root "$release"
chmod -R go-w "$release"
ln -s "releases/$version" /opt/dakia/current
install -m 644 "$release/dakia.service" /etc/systemd/system/dakia.service
install -d -m 700 /etc/dakia
install -m 600 /dev/null /etc/dakia/dakia.env
printf 'DAKIA_ORIGIN=%s\nDAKIA_DATA_DIR=/var/lib/dakia/data\nDAKIA_CREDENTIAL_KEY_FILE=/var/lib/dakia-secrets/credential-key\n' "$origin" > /etc/dakia/dakia.env
chmod 600 /etc/dakia/dakia.env
systemd-analyze verify /etc/systemd/system/dakia.service
systemctl daemon-reload
systemctl enable --now dakia
for _ in $(seq 1 30); do
    if /opt/dakia/current/dakia-healthcheck >/dev/null 2>&1; then break; fi
    sleep 1
done
/opt/dakia/current/dakia-healthcheck
printf '\nInstalled version: '; cat "$release/VERSION"
printf 'Source revision: '; cat "$release/REVISION"
msg_ok "Native runtime is healthy"
motd_ssh
customize
cleanup_lxc
