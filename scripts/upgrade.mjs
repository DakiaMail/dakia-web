// Runs in the installed image. No Docker socket, shell evaluation, or host runtime needed.
import fs from 'node:fs';
import path from 'node:path';
export const repository = 'DakiaMail/dakia-web';
export const imagePrefix = 'ghcr.io/dakiamail/dakia-web@sha256:';
export function version(tag) {
  const m = /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/.exec(tag);
  if (!m || m.slice(1, 4).some(x => BigInt(x) > 18446744073709551615n) || m[4]?.split('.').some(x => /^\d+$/.test(x) && x.length > 1 && x[0] === '0')) return null;
  return [BigInt(m[1]), BigInt(m[2]), BigInt(m[3]), m[4]?.split('.') || []];
}
export function compare(a, b) {
  const x = version(a), y = version(b);
  if (!x || !y) throw Error('Invalid release version');
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i] ? 1 : -1;
  if (!x[3].length || !y[3].length) return !x[3].length && !y[3].length ? 0 : !x[3].length ? 1 : -1;
  for (let i = 0; i < Math.max(x[3].length, y[3].length); i++) {
    const a = x[3][i], b = y[3][i];
    if (a === b) continue;
    if (a === undefined || b === undefined) return a === undefined ? -1 : 1;
    const an = /^\d+$/.test(a), bn = /^\d+$/.test(b);
    if (an && bn) return BigInt(a) > BigInt(b) ? 1 : -1;
    if (an !== bn) return an ? -1 : 1;
    return a > b ? 1 : -1;
  }
  return 0;
}
export function select(releases, explicit) {
  let candidates = releases.filter(r => !r.draft && version(r.tag_name) && r.prerelease === !!version(r.tag_name)[3].length);
  if (explicit) candidates = candidates.filter(r => r.tag_name === explicit);
  else if (candidates.some(r => !r.prerelease)) candidates = candidates.filter(r => !r.prerelease);
  return candidates.sort((a, b) => compare(b.tag_name, a.tag_name))[0];
}
export function validate(m, tag) {
  if (m.format !== 1 || m.version !== tag || !version(tag) || !/^[a-f0-9]{40}$/.test(m.revision)
    || !m.image?.startsWith(imagePrefix) || !/^[a-f0-9]{64}$/.test(m.image.slice(imagePrefix.length))) throw Error('Unsupported or invalid release manifest');
  return m;
}
async function json(url, limit) {
  const response = await fetch(url, { headers: { 'User-Agent': 'Dakia upgrade command' }, signal: AbortSignal.timeout(15000) });
  if (!response.ok) throw Error(`Release request failed (${response.status})`);
  const chunks = []; let size = 0;
  for await (const chunk of response.body) { size += chunk.length; if (size > limit) throw Error('Release response too large'); chunks.push(Buffer.from(chunk)); }
  return JSON.parse(Buffer.concat(chunks).toString('utf8'));
}
export async function discover(explicit) {
  if (explicit && !version(explicit)) throw Error('Use a version such as v1.2.3');
  const releases = [];
  if (explicit) releases.push(await json(`https://api.github.com/repos/${repository}/releases/tags/${explicit}`, 4 * 1024 * 1024));
  else for (let page = 1; page <= 10; page++) {
    const batch = await json(`https://api.github.com/repos/${repository}/releases?per_page=100&page=${page}`, 4 * 1024 * 1024);
    if (!Array.isArray(batch)) throw Error('Invalid releases response');
    releases.push(...batch);
    if (batch.length < 100) break;
    if (page === 10) throw Error('Release list is too large to select safely. Specify --version.');
  }
  const release = select(releases, explicit);
  if (!release) throw Error('No published release is available yet. Your installation has not changed.');
  const url = `https://github.com/${repository}/releases/download/${release.tag_name}/dakia-release.json`;
  if (!release.assets?.some(a => a.name === 'dakia-release.json' && a.browser_download_url === url)) throw Error('This release has no verified upgrade manifest');
  return { ...validate(await json(url, 16384), release.tag_name), notes_url: `https://github.com/${repository}/releases/tag/${release.tag_name}` };
}
const root = '/installation';
const work = `${root}/.dakia-upgrades/work`;
const read = name => JSON.parse(fs.readFileSync(`${work}/${name}.json`, 'utf8'));
function write(name, value) { fs.writeFileSync(`${work}/${name}.json`, JSON.stringify(value, null, 2), { mode: 0o600 }); }
function assert(condition, message) { if (!condition) throw Error(message); }
export function preflight(config, container, hostRoot) {
  const service = config.services?.dakia;
  assert(service && container.State.Running, 'Start the existing Dakia installation before upgrading');
  assert(config.name === container.Config.Labels['com.docker.compose.project'], 'Compose project differs from the running installation');
  assert(container.Config.Labels['com.docker.compose.service'] === 'dakia' && !container.Config.Labels['com.docker.compose.oneoff']?.match(/true/i), 'Expected the Dakia service container');
  assert(service.image === container.Config.Image, 'Compose image differs from the running installation. Resolve overrides before upgrading');
  const env = service.environment || {};
  assert(!env.DAKIA_DATA_DIR || env.DAKIA_DATA_DIR === '/data', 'Custom data paths need a manual upgrade');
  assert(!env.DAKIA_CREDENTIAL_KEY_FILE || env.DAKIA_CREDENTIAL_KEY_FILE === '/secrets/credential-key', 'Custom key paths need a manual upgrade');
  for (const target of ['/data', '/secrets']) {
    const effective = service.volumes?.find(v => v.target === target);
    const actual = container.Mounts.find(v => v.Destination === target);
    assert(effective && actual && ['volume', 'bind'].includes(actual.Type) && actual.RW, `Expected a persistent writable ${target} mount`);
    assert(effective.type === actual.Type, 'Compose mount type differs from the running installation');
    const source = effective.type === 'volume' ? config.volumes?.[effective.source]?.name : effective.source;
    assert(source === (actual.Type === 'volume' ? actual.Name : actual.Source), 'Compose mount differs from the running installation');
    const backupRoot = `${hostRoot}/.dakia-upgrades`;
    assert(!(actual.Source === backupRoot || actual.Source.startsWith(`${backupRoot}/`) || backupRoot.startsWith(`${actual.Source}/`)), 'Backup directory overlaps application storage');
  }
  for (const mount of container.Mounts) {
    if (['/data', '/secrets'].includes(mount.Destination) || mount.Type === 'tmpfs') continue;
    assert(!mount.RW && mount.Type === 'bind', 'Extra writable or named mounts require a manual upgrade');
  }
  const files = container.Config.Labels['com.docker.compose.project.config_files']?.split(',');
  assert(files?.length && files.every(f => path.dirname(f) === hostRoot), 'Keep Compose files beside .env for this upgrade command');
  assert(container.Config.Labels['com.docker.compose.project.working_dir'] === hostRoot, 'Run from the original Compose project directory');
  assert(!service.user || ['1000', '1000:1000'].includes(service.user), 'Custom service users require a manual upgrade');
  assert(!service.healthcheck?.disable && !service.healthcheck?.test?.includes('NONE'), 'Enable the Docker healthcheck before upgrading');
  assert(!service.build && !service.scale && !service.deploy?.replicas, 'Builds or scaled services require a manual upgrade');
  return { project: config.name, files, previous_image: container.Image, configured_image: service.image, mounts: container.Mounts };
}
export function pin(text, image) {
  assert(/^ghcr\.io\/dakiamail\/dakia-web@sha256:[a-f0-9]{64}$/.test(image), 'Invalid image pin');
  const lines = text.split('\n');
  const matches = lines.map((line, index) => /^\s*(?:export\s+)?DAKIA_IMAGE\s*=/.test(line) ? index : -1).filter(i => i >= 0);
  assert(matches.length <= 1, 'Duplicate DAKIA_IMAGE settings in .env');
  if (matches.length) lines[matches[0]] = `DAKIA_IMAGE=${image}`;
  else { if (lines.at(-1) === '') lines.pop(); lines.push(`DAKIA_IMAGE=${image}`, ''); }
  return lines.join('\n');
}
const quote = text => `'${text.replaceAll("'", "'\\''")}'`;
async function main(command, args) {
  if (command === 'discover') write('target', await discover(args[0] || undefined));
  else if (command === 'preflight') {
    const p = preflight(read('config'), read('container')[0], args[0]);
    assert(fs.existsSync(`${root}/.env`) && !fs.lstatSync(`${root}/.env`).isSymbolicLink(), 'A regular .env file is required');
    assert(!fs.existsSync(`${root}/.env.dakia-upgrade`), 'Inspect the stale .env.dakia-upgrade file before upgrading');
    pin(fs.readFileSync(`${root}/.env`, 'utf8'), `${imagePrefix}${'0'.repeat(64)}`);
    write('previous', p);
  } else if (command === 'check-mounts') {
    const p = read('previous');
    assert(fs.readFileSync('/secrets/credential-key').length === 32, 'Credential key is missing or invalid');
    let bytes = 0;
    const measure = file => {
      const stat = fs.lstatSync(file);
      assert(!stat.isSymbolicLink(), 'Symlinked application files need a manual backup');
      if (stat.isDirectory()) for (const name of fs.readdirSync(file)) measure(path.join(file, name));
      else { assert(stat.isFile(), 'Special application files need a manual backup'); bytes += stat.size; }
    };
    for (const file of ['/data/control.sqlite', '/data/accounts', '/data/durable', '/secrets']) if (fs.existsSync(file)) measure(file);
    for (const m of p.mounts.filter(m => !['/data', '/secrets'].includes(m.Destination) && m.Type !== 'tmpfs')) {
      assert(fs.statSync(m.Destination).isFile(), 'Extra directory mounts require a manual backup'); measure(m.Destination);
    }
    const disk = fs.statfsSync(root);
    assert(disk.bavail * disk.bsize > bytes * 2 + 64 * 1024 * 1024, 'Insufficient backup space. Keep room for the backup and a separate restore');
  } else if (command === 'get') {
    const value = read(args[0])[args[1]];
    assert(typeof value === 'string' && !/[\r\n\0]/.test(value), 'Invalid field');
    console.log(value);
  } else if (command === 'validate-pin') {
    const before = read('config'), after = read('pinned');
    const target = read('target');
    assert(after.services.dakia.image === target.image, 'An override prevents DAKIA_IMAGE from selecting the release');
    after.services.dakia.image = before.services.dakia.image;
    assert(JSON.stringify(before) === JSON.stringify(after), 'Pinning changed configuration beyond the image');
  } else if (command === 'pin') {
    const file = `${root}/.env`, tmp = `${root}/.env.dakia-upgrade`;
    assert(!fs.existsSync(tmp), 'Remove the stale .env.dakia-upgrade file after inspecting it');
    fs.writeFileSync(tmp, pin(fs.readFileSync(file, 'utf8'), read('target').image), { mode: fs.statSync(file).mode & 0o777, flag: 'wx' });
    fs.renameSync(tmp, file);
  } else if (command === 'prepare') {
    const backup = `${root}/${args[0]}`;
    fs.mkdirSync(backup, { mode: 0o700 });
    const p = read('previous');
    fs.mkdirSync(`${backup}/deployment`, { mode: 0o700 });
    for (const f of [...p.files.map(f => path.basename(f)), '.env']) {
      assert(!fs.lstatSync(`${root}/${f}`).isSymbolicLink(), 'Symlinked configuration needs a manual backup');
      fs.copyFileSync(`${root}/${f}`, `${backup}/deployment/${f}`, fs.constants.COPYFILE_EXCL);
      fs.chmodSync(`${backup}/deployment/${f}`, 0o600);
    }
    for (const name of ['config', 'container', 'previous', 'target']) fs.copyFileSync(`${work}/${name}.json`, `${backup}/${name}.json`);
    const hostBackup = `${args[1]}/${args[0]}`;
    const override = { services: { dakia: { image: p.previous_image, pull_policy: 'never', volumes: [
      { type: 'bind', source: `${hostBackup}/restored/data`, target: '/data' },
      { type: 'bind', source: `${hostBackup}/secrets`, target: '/secrets' },
      ...p.mounts.filter(m => !['/data', '/secrets'].includes(m.Destination) && m.Type !== 'tmpfs').map((m, i) => ({ type: 'bind', source: `${hostBackup}/extra-mounts/${i}`, target: m.Destination, read_only: true }))
    ] } } };
    fs.writeFileSync(`${backup}/recovery.compose.json`, JSON.stringify(override, null, 2), { mode: 0o600 });
    const compose = `docker compose --project-directory ${quote(args[1])} --env-file ${quote(`${hostBackup}/deployment/.env`)} -p ${quote(p.project)} ${p.files.map(f => `-f ${quote(`${hostBackup}/deployment/${path.basename(f)}`)}`).join(' ')} -f ${quote(`${hostBackup}/recovery.compose.json`)}`;
    const recovery = `#!/bin/sh\nset -eu\numask 077\nbackup=${quote(hostBackup)}\n[ -f "$backup/data/snapshot/manifest.json" ] || { echo 'Backup is incomplete'; exit 1; }\n[ ! -e "$backup/restored" ] || { echo 'Restore directory already exists. Inspect it before retrying.'; exit 1; }\nmkdir "$backup/restored"\ndocker run --rm --network none --user 0 --read-only --tmpfs /tmp --mount "type=bind,src=$backup,dst=/backup,readonly" --mount "type=bind,src=$backup/restored,dst=/restored" --entrypoint /bin/sh ${quote(p.previous_image)} -c '/app/dakia-admin restore /backup/data/snapshot /restored/data /backup/secrets/credential-key && chown -R 1000:1000 /restored/data'\nprintf '%s\\n' 'Restore prepared. Failed data has not been replaced.' 'After reviewing the recovery guide, activate with:' ${quote(`${compose} up -d --no-deps --pull never dakia`)} 'Background jobs remain disabled. Reconcile mail actions before enabling them.'\n`;
    fs.writeFileSync(`${backup}/recover.sh`, recovery, { mode: 0o700 });
  } else if (command === 'backup-files') {
    const p = JSON.parse(fs.readFileSync('/upgrade/previous.json', 'utf8'));
    assert(fs.readFileSync('/secrets/credential-key').length === 32, 'Credential key is missing or invalid');
    fs.cpSync('/secrets', '/upgrade/secrets', { recursive: true, dereference: false, errorOnExist: true, force: false });
    const own = dir => { for (const name of fs.readdirSync(dir)) { const file = path.join(dir, name); const stat = fs.lstatSync(file); assert(!stat.isSymbolicLink(), 'Symlinked secrets need a manual backup'); if (stat.isDirectory()) own(file); fs.chownSync(file, 1000, 1000); } fs.chownSync(dir, 1000, 1000); };
    own('/upgrade/secrets');
    fs.mkdirSync('/upgrade/data', { mode: 0o700 });
    fs.chownSync('/upgrade/data', 1000, 1000);
    fs.mkdirSync('/upgrade/extra-mounts', { mode: 0o700 });
    p.mounts.filter(m => !['/data', '/secrets'].includes(m.Destination) && m.Type !== 'tmpfs').forEach((m, i) => {
      assert(fs.statSync(m.Destination).isFile(), 'Extra directory mounts require a manual backup');
      fs.copyFileSync(m.Destination, `/upgrade/extra-mounts/${i}`);
      fs.chmodSync(`/upgrade/extra-mounts/${i}`, 0o600);
      fs.chownSync(`/upgrade/extra-mounts/${i}`, 1000, 1000);
    });
  } else throw Error('Unknown upgrade helper operation');
}
if (process.argv[1] && path.resolve(process.argv[1]) === new URL(import.meta.url).pathname) {
  main(process.argv[2], process.argv.slice(3)).catch(error => { console.error(error.message); process.exitCode = 1; });
}
