#!/bin/sh
# Host orchestration only. JSON and HTTPS run inside the installed Dakia image.
set -eu
umask 077
fail() { printf '%s\n' "$*" >&2; exit 1; }
check=false
yes=false
release=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --check) check=true ;;
    --yes) yes=true ;;
    --version) shift; [ "$#" -gt 0 ] || fail 'Missing release version'; release=$1 ;;
    --help|-h) printf '%s\n' 'Usage: sh scripts/upgrade.sh [--check] [--version vX.Y.Z] [--yes]'; exit 0 ;;
    *) fail "Unknown option: $1" ;;
  esac
  shift
done
command -v docker >/dev/null || fail 'Docker is required'
docker compose version >/dev/null || fail 'Docker Compose is required'
[ -z "${DAKIA_IMAGE+x}" ] || fail 'Unset the shell DAKIA_IMAGE override first; the upgrade saves the image in .env'
[ -z "${COMPOSE_FILE:-}" ] && [ -z "${COMPOSE_ENV_FILES:-}" ] || fail 'Custom Compose file or env-file selection needs a manual upgrade'
[ -z "${DOCKER_HOST:-}" ] || fail 'Use a local Docker context without DOCKER_HOST for this upgrade'
endpoint=$(docker context inspect --format '{{.Endpoints.docker.Host}}')
case "$endpoint" in unix://*) ;; *) fail 'Run the upgrade on the Docker host using a local context' ;; esac
root=$(pwd -P)
tool=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
case "$root$tool" in *,*) fail 'Paths containing commas need a manual upgrade' ;; esac
[ -f "$tool/upgrade.mjs" ] || fail 'Keep upgrade.sh and upgrade.mjs together'
[ -f .env ] && [ ! -L .env ] || fail 'Run from the installation directory containing a regular .env file'
[ ! -L .dakia-upgrades ] || fail '.dakia-upgrades must not be a symlink'
mkdir -p .dakia-upgrades
chmod 700 .dakia-upgrades
mkdir .dakia-upgrades/lock 2>/dev/null || fail 'Another upgrade holds .dakia-upgrades/lock. If interrupted, inspect its progress before removing this lock.'
stopped=false
activated=false
backup=
cleanup() {
  result=$?
  trap - EXIT HUP INT TERM
  if [ "$stopped" = true ] && [ "$activated" = false ]; then
    printf '%s\n' 'Upgrade stopped before activation. Restarting the unchanged container.' >&2
    docker start "$container" >/dev/null 2>&1 || printf '%s\n' 'Restart failed. Start the previous container manually.' >&2
  fi
  if [ "$result" -ne 0 ] && [ "$activated" = true ]; then
    docker compose stop dakia >/dev/null 2>&1 || printf '%s\n' 'Could not stop Dakia. Stop the failed container before recovery.' >&2
    printf '%s\n' 'The new version was stopped. Do not start the old image against its data.' "Backup and progress: $backup" "Prepare a separate restore with: sh '$backup/recover.sh'" 'Follow docs/upgrading.md before activating the restored installation.' >&2
  fi
  rmdir .dakia-upgrades/lock 2>/dev/null || true
  exit "$result"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' HUP TERM
if [ -f .dakia-upgrades/active ]; then
  fail 'An earlier upgrade needs attention. Read .dakia-upgrades/active and its progress file. Follow docs/upgrading.md before clearing it.'
fi
[ ! -L .dakia-upgrades/work ] || fail 'Upgrade work directory must not be a symlink'
mkdir -p .dakia-upgrades/work
container=$(docker compose ps -q dakia)
[ -n "$container" ] || fail 'Start the existing Dakia service before upgrading'
case "$container" in *'
'*) fail 'Only one Dakia container is supported' ;; esac
current=$(docker inspect --format '{{.Image}}' "$container")
helper() {
  docker run --rm --read-only --tmpfs /tmp --user "$(id -u):$(id -g)" \
    --mount "type=bind,src=$root,dst=/installation" \
    --mount "type=bind,src=$tool,dst=/tool,readonly" \
    --entrypoint node "$current" /tool/upgrade.mjs "$@"
}
docker compose config --format json > .dakia-upgrades/work/config.json 2> .dakia-upgrades/work/compose-errors.log || fail 'Compose configuration failed. Inspect the private .dakia-upgrades/work/compose-errors.log'
docker inspect "$container" > .dakia-upgrades/work/container.json
helper preflight "$root"
docker run --rm --network none --read-only --user 0 --volumes-from "$container":ro \
  --mount "type=bind,src=$root,dst=/installation,readonly" --mount "type=bind,src=$tool,dst=/tool,readonly" \
  --entrypoint node "$current" /tool/upgrade.mjs check-mounts
printf '%s\n' 'Checking published Dakia releases…'
helper discover "$release"
target=$(helper get target image)
version=$(helper get target version)
notes=$(helper get target notes_url)
installed=$(docker image inspect --format '{{index .Config.Labels "org.opencontainers.image.version"}}' "$current")
case "$installed" in v[0-9]*) ;; *) installed=$current ;; esac
printf 'Installed: %s\nAvailable: %s\nRelease notes: %s\n' "${installed:-$current}" "$version" "$notes"
# Validate the pin and all resolved settings before any downtime or .env edit.
DAKIA_IMAGE="$target" docker compose config --format json > .dakia-upgrades/work/pinned.json 2> .dakia-upgrades/work/compose-errors.log || fail 'Image selection failed. Inspect the private .dakia-upgrades/work/compose-errors.log'
helper validate-pin
# Compare semantic release versions, treating older unlabelled images as unknown.
comparison=$(docker run --rm --network none --read-only --mount "type=bind,src=$tool,dst=/tool,readonly" --entrypoint node "$current" --input-type=module -e \
  'import {version,compare} from "/tool/upgrade.mjs"; const [a,b]=process.argv.slice(1); console.log(version(a)?compare(b,a):"unknown")' "$installed" "$version")
if [ "$comparison" = -1 ]; then
  if [ -z "$release" ]; then printf '%s\n' 'There is no newer release on this channel.'; exit 0; fi
  fail 'Downgrades require recovery from a matching backup'
fi
if [ "$check" = true ]; then exit 0; fi
local_target=$(docker image inspect --format '{{.Id}}' "$target" 2>/dev/null || true)
if [ "$current" = "$local_target" ]; then printf '%s\n' 'This release is already installed.'; exit 0; fi
if [ "$yes" = false ]; then
  printf '%s\n' 'Dakia will briefly stop while a backup is made and the release starts.'
  printf 'Continue? [y/N] '
  read -r answer || exit 0
  case "$answer" in y|Y|yes|YES) ;; *) exit 0 ;; esac
fi
docker pull "$target"
new_image=$(docker image inspect --format '{{.Id}}' "$target")
if [ "$current" = "$new_image" ]; then printf '%s\n' 'This release is already installed.'; exit 0; fi
backup_rel=".dakia-upgrades/$(date -u +%Y%m%dT%H%M%SZ)-$$"
backup="$root/$backup_rel"
helper prepare "$backup_rel" "$root"
printf '%s\n' "$backup" > .dakia-upgrades/active
progress() { printf '%s\n' "$1" > "$backup/progress"; }
progress prepared
printf 'Backup: %s\n' "$backup"
# Retain the current image even if the operator later prunes untagged images.
docker image tag "$current" "dakia-upgrade-backup:$(basename "$backup")"
stopped=true
progress stopping
docker compose stop dakia
progress backing-up
docker run --rm --network none --read-only --tmpfs /tmp --user 0 --volumes-from "$container":ro \
  --mount "type=bind,src=$backup,dst=/upgrade" --mount "type=bind,src=$tool,dst=/tool,readonly" \
  --entrypoint node "$current" /tool/upgrade.mjs backup-files
# Store::open takes a lock and may checkpoint SQLite, so /data must be writable.
docker run --rm --network none --read-only --tmpfs /tmp --user 1000:1000 --volumes-from "$container" \
  --mount "type=bind,src=$backup/data,dst=/upgrade" --entrypoint /app/dakia-admin "$current" \
  backup /data /upgrade/snapshot /secrets/credential-key > "$backup/backup.log" 2>&1
progress backed-up
# Mark activation before changing the pin: interruption requires explicit inspection.
activated=true
progress activating
helper pin
docker compose config --format json > .dakia-upgrades/work/pinned.json
helper validate-pin
docker compose up -d --no-deps --pull never dakia > "$backup/start.log" 2>&1
new_container=$(docker compose ps -aq dakia)
[ -n "$new_container" ] || fail 'The upgraded container was not created'
ready=false
attempt=0
while [ "$attempt" -lt 36 ]; do
  actual=$(docker inspect --format '{{.Image}}' "$new_container")
  state=$(docker inspect --format '{{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{end}}' "$new_container")
  if [ "$actual" = "$new_image" ] && [ "$state" = 'running healthy' ] && docker exec "$new_container" /app/dakia-healthcheck >/dev/null 2>&1; then ready=true; break; fi
  case "$state" in exited*|dead*|*unhealthy*) break ;; esac
  attempt=$((attempt + 1))
  sleep 5
done
[ "$ready" = true ] || fail 'The new release did not pass startup checks'
progress complete
rm .dakia-upgrades/active
printf 'Dakia %s is running. Backup: %s\n' "$version" "$backup"
printf '%s\n' 'Check your HTTPS address, sign in, and verify mailbox access.'
