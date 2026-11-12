#!/bin/sh
# Installs Dakia on a fresh Debian/Ubuntu server: Docker, Compose, and HTTPS.
# Run as root: sh install.sh --domain mail.example.com
# Cloud-init user data can call this same script; see deploy/cloud-init.yaml.
set -eu
umask 077
fail() { printf '%s\n' "$*" >&2; exit 1; }
note() { printf '%s\n' "$*"; }
usage() {
  cat <<'EOF'
Usage: sh install.sh --domain mail.example.com [--dir /opt/dakia] [--ref main] [--prepare-only]

  --domain        Exact HTTPS domain, without a scheme or path. Point its A record
                  at this server before running, and keep ports 80 and 443 reachable.
  --dir           Installation directory for Compose files and .env. Default /opt/dakia.
  --ref           Repository ref to download Compose files from when not run from a
                  checkout. Default main.
  --prepare-only  Write the Compose files and .env without installing Docker or starting
                  the stack.
EOF
}
domain=
dir=/opt/dakia
ref=main
prepare_only=false
while [ "$#" -gt 0 ]; do
  case "$1" in
    --domain) shift; [ "$#" -gt 0 ] || fail 'Missing --domain value'; domain=$1 ;;
    --dir) shift; [ "$#" -gt 0 ] || fail 'Missing --dir value'; dir=$1 ;;
    --ref) shift; [ "$#" -gt 0 ] || fail 'Missing --ref value'; ref=$1 ;;
    --prepare-only) prepare_only=true ;;
    --help|-h) usage; exit 0 ;;
    *) fail "Unknown option: $1" ;;
  esac
  shift
done
[ -n "$domain" ] || fail 'Set the HTTPS domain people will open, for example: --domain mail.example.com'
case "$domain" in
  *://*) fail 'Give the domain without a scheme: mail.example.com' ;;
  */*) fail 'Give the domain without a path: mail.example.com' ;;
  *:*) fail 'Give the domain without a port; Dakia is served on 443' ;;
esac
printf '%s' "$domain" | grep -Eq '^[A-Za-z0-9]([A-Za-z0-9.-]*[A-Za-z0-9])$' || fail "Not a valid domain: $domain"
origin="https://$domain"
case "$dir" in
  /*) ;;
  *) fail 'Use an absolute --dir path' ;;
esac
if [ "$prepare_only" = false ] && [ "$(id -u)" != 0 ]; then
  fail 'Run as root, or use --prepare-only to write the files only'
fi

# Compose files come from the checkout this script lives in, or from the repository.
here=$(dirname -- "$0")
here=$(CDPATH='' cd -- "$here" && pwd -P)
checkout=$(dirname -- "$here")
copy_or_fetch() {
  source_path=$1
  target_path=$2
  if [ -f "$checkout/$source_path" ]; then
    cp "$checkout/$source_path" "$target_path"
  else
    command -v curl >/dev/null || fail 'Install curl to download the Compose files'
    url="https://raw.githubusercontent.com/DakiaMail/dakia-web/$ref/$source_path"
    curl -fsSL --max-time 60 "$url" -o "$target_path" \
      || fail "Could not download $source_path from $url"
  fi
  [ -s "$target_path" ] || fail "$source_path came back empty; check the ref"
}

note "Preparing $dir for $origin"
mkdir -p "$dir/deploy" "$dir/scripts"
copy_or_fetch compose.yaml "$dir/compose.yaml"
copy_or_fetch compose.caddy.yaml "$dir/compose.caddy.yaml"
copy_or_fetch deploy/Caddyfile "$dir/deploy/Caddyfile"
copy_or_fetch scripts/upgrade.sh "$dir/scripts/upgrade.sh"
copy_or_fetch scripts/upgrade.mjs "$dir/scripts/upgrade.mjs"
if [ -f "$dir/.env" ]; then
  note "Keeping the existing $dir/.env; delete it to change the address"
else
  printf 'DAKIA_ORIGIN=%s\n' "$origin" > "$dir/.env"
  chmod 600 "$dir/.env"
fi

if [ "$prepare_only" = true ]; then
  note "Wrote $dir/compose.yaml, $dir/compose.caddy.yaml, $dir/deploy/Caddyfile and $dir/.env"
  note "Start with: cd $dir && docker compose -f compose.yaml -f compose.caddy.yaml up -d --wait"
  exit 0
fi

command -v curl >/dev/null || fail 'Install curl first: apt-get update && apt-get install -y curl'
if ! command -v docker >/dev/null; then
  note 'Installing Docker'
  curl -fsSL https://get.docker.com | sh
fi
docker compose version >/dev/null 2>&1 || fail 'Docker Compose plugin is missing; install docker-compose-plugin'

note 'Starting Dakia with HTTPS'
cd "$dir"
docker compose -f compose.yaml -f compose.caddy.yaml up -d --wait --wait-timeout 900 \
  || fail 'Compose did not start. If another service already holds ports 80 or 443, stop it and retry, or use the reverse-proxy option in docs/hosting.md.'

if curl -fsS --max-time 10 -o /dev/null http://127.0.0.1:3025/healthz; then
  note 'Dakia is running'
else
  note 'Dakia started but did not answer its health check yet; check: docker compose logs dakia'
fi
if command -v getent >/dev/null && getent hosts "$domain" >/dev/null 2>&1; then
  note "Open $origin and create your login now. The first login becomes the owner, so create it before you share the address."
  note 'Caddy may need a minute to obtain the certificate on your first visit.'
else
  note "$domain does not resolve yet. Point its A record at this server; Caddy keeps retrying and issues the certificate once DNS is correct."
  note "Then open $origin and create your login, which becomes the owner."
fi
note "Upgrade later with: cd $dir && sh scripts/upgrade.sh"
note "Keep both Dakia volumes and the caddy_data volume; losing /secrets loses access to stored mailbox credentials."
