// Opt-in integration: two real published images, fresh project/volumes, no mail networking.
// node scripts/tests/upgrade-docker.mjs OLD_IMAGE NEW_IMAGE
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const tools = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const [oldImage, newImage] = process.argv.slice(2);
assert(oldImage?.startsWith('ghcr.io/dakiamail/dakia-web@sha256:') && newImage?.startsWith('ghcr.io/dakiamail/dakia-web@sha256:'));
const root = fs.mkdtempSync(path.join(fs.realpathSync(os.tmpdir()), 'dakia-upgrade-docker-'));
const project = `dakia-upgrade-e2e-${process.pid}`;
function run(cmd, args, options = {}) {
  const r = spawnSync(cmd, args, { cwd: root, encoding: 'utf8', timeout: 240000, maxBuffer: 1024 * 1024, ...options });
  if (r.status !== 0) throw Error(`${cmd} ${args[0]} failed: ${r.stderr}\n${r.stdout}`);
  return r.stdout.trim();
}
const docker = (...args) => run('docker', args);
const compose = (...args) => docker('compose', ...args);
const node = (image, mounts, code) => docker('run', '--rm', '--network', 'none', '--user', '0', ...mounts, '--entrypoint', 'node', image, '--input-type=module', '-e', code);
try {
  // Images are pre-pulled by the caller. All application containers have networking disabled.
  fs.mkdirSync(`${root}/scripts`);
  fs.copyFileSync(`${tools}/upgrade.sh`, `${root}/scripts/upgrade.sh`);
  const helper = fs.readFileSync(`${tools}/upgrade.mjs`, 'utf8').replace("write('target', await discover(args[0] || undefined))", "write('target', JSON.parse(fs.readFileSync('/tool/fixture-release.json', 'utf8')))");
  fs.writeFileSync(`${root}/scripts/upgrade.mjs`, helper);
  fs.writeFileSync(`${root}/scripts/fixture-release.json`, JSON.stringify({ format: 1, version: 'v0.2.0', revision: 'a'.repeat(40), image: newImage, notes_url: 'https://github.com/DakiaMail/dakia-web/releases/tag/v0.2.0' }));
  fs.writeFileSync(`${root}/.env`, 'DAKIA_ORIGIN=https://mail.test\nDAKIA_RELEASE_CHECKS=false\n');
  fs.writeFileSync(`${root}/compose.yaml`, `name: ${project}\nservices:\n  dakia:\n    image: \${DAKIA_IMAGE:-${oldImage}}\n    network_mode: none\n    environment:\n      DAKIA_ORIGIN: \${DAKIA_ORIGIN}\n      DAKIA_RELEASE_CHECKS: 'false'\n    volumes:\n      - data:/data\n      - secrets:/secrets\n    read_only: true\n    tmpfs: [/tmp]\nvolumes:\n  data:\n  secrets:\n`);
  compose('up', '-d', '--wait', '--wait-timeout', '100');
  let cid = compose('ps', '-q', 'dakia');
  docker('exec', cid, 'node', '--input-type=module', '-e', `import fs from 'node:fs';
import os from 'node:os'; const r=await fetch('http://127.0.0.1:3025/auth/login',{method:'POST',headers:{Origin:'https://mail.test'},body:new URLSearchParams({username:'upgrade-fixture',password:'synthetic-disposable-password'}),redirect:'manual'});if(r.status!==303)throw Error('setup failed');fs.writeFileSync('/data/fixture-cookie',r.headers.get('set-cookie').split(';')[0]);`);
  const key = docker('exec', cid, 'sha256sum', '/secrets/credential-key');
  compose('stop', 'dakia');
  const mount = ['--volumes-from', cid];
  const oldCount = node(oldImage, mount, `import {DatabaseSync} from 'node:sqlite';import fs from 'node:fs';
import os from 'node:os';import {createCipheriv,randomBytes} from 'node:crypto';const db=new DatabaseSync('/data/control.sqlite'); const user=db.prepare('SELECT user_id FROM local_login WHERE is_owner=1').get().user_id;const id='11111111-1111-4111-8111-111111111111';const a={id,email:'fixture@example.invalid',display_name:'Disposable mailbox',provider:'imap',state:'ready',revision:1};const input={email:a.email,display_name:a.display_name,provider:'imap',imap_host:'fixture.invalid',imap_port:993,imap_tls:'implicit',smtp_host:'fixture.invalid',smtp_port:465,smtp_tls:'implicit',username:a.email,password:'synthetic-only'};const nonce=randomBytes(12);const cipher=createCipheriv('aes-256-gcm',fs.readFileSync('/secrets/credential-key'),nonce);cipher.setAAD(Buffer.from(user+':'+id));const sealed=Buffer.concat([nonce,cipher.update(JSON.stringify(input)),cipher.final(),cipher.getAuthTag()]);const columns=db.prepare('PRAGMA table_info(accounts)').all();if(columns.some(c=>c.name==='connected_at'))db.prepare('INSERT INTO accounts(id,user_id,metadata,credential,connected_at) VALUES(?,?,?,?,?)').run(id,user,JSON.stringify(a),sealed,new Date().toISOString());else db.prepare('INSERT INTO accounts(id,user_id,metadata,credential) VALUES(?,?,?,?)').run(id,user,JSON.stringify(a),sealed);db.prepare("INSERT OR REPLACE INTO settings(key,value) VALUES('jobs_enabled','false')").run();console.log(db.prepare('SELECT count(*) AS n FROM _sqlx_migrations').get().n);`);
  compose('start', 'dakia');
  // A wrapper only skips pulling an already present immutable fixture image.
  const realDocker = run('which', ['docker']); fs.mkdirSync(`${root}/bin`);
  fs.writeFileSync(`${root}/bin/docker`, `#!/bin/sh\nif [ "$1" = pull ]; then exec '${realDocker}' image inspect "$2" --format '{{.Id}}'; fi\nexec '${realDocker}' "$@"\n`, { mode: 0o700 });
  const env = { ...process.env, PATH: `${root}/bin:${process.env.PATH}` };
  for (const k of ['DAKIA_IMAGE','COMPOSE_FILE','COMPOSE_ENV_FILES','DOCKER_HOST']) delete env[k];
  const output = run('sh', ['scripts/upgrade.sh', '--yes'], { env });
  assert.match(output, /is running/);
  assert(!output.includes('synthetic-disposable-password'));
  cid = compose('ps', '-q', 'dakia');
  assert.equal(docker('exec', cid, 'sha256sum', '/secrets/credential-key'), key);
  const after = docker('exec', cid, 'node', '--input-type=module', '-e', `import fs from 'node:fs';
import os from 'node:os';const cookie=fs.readFileSync('/data/fixture-cookie','utf8');const base='http://127.0.0.1:3025';for(const endpoint of ['/api/session','/api/accounts']){const r=await fetch(base+endpoint,{headers:{Cookie:cookie}});if(r.status!==200)throw Error(endpoint);const value=await r.json();if(endpoint.endsWith('accounts')&&(value.length!==1||value[0].email!=='fixture@example.invalid'))throw Error('mailbox changed');}console.log('owner and mailbox preserved');`);
  compose('up', '-d', '--no-deps', '--pull', 'never', 'dakia');
  assert.equal(docker('inspect', '--format', '{{.Image}}', cid), docker('image', 'inspect', '--format', '{{.Id}}', newImage));
  const backupName = fs.readdirSync(`${root}/.dakia-upgrades`).find(x => /^\d/.test(x));
  const backup = `${root}/.dakia-upgrades/${backupName}`;
  compose('stop', 'dakia');
  const newCount = node(newImage, ['--volumes-from', cid], `import {DatabaseSync} from 'node:sqlite';const db=new DatabaseSync('/data/control.sqlite');console.log(db.prepare('SELECT count(*) AS n FROM _sqlx_migrations').get().n);`);
  assert(Number(newCount) > Number(oldCount), `Expected a migration: ${oldCount} -> ${newCount}`);
  const recovered = run('sh', [`${backup}/recover.sh`]);
  assert.match(recovered, /Restore prepared/);
  const restoreCheck = node(oldImage, ['--mount', `type=bind,src=${backup}/restored/data,dst=/data`, '--mount', `type=bind,src=${backup}/secrets,dst=/secrets`], `import {DatabaseSync} from 'node:sqlite';const db=new DatabaseSync('/data/control.sqlite');if(db.prepare('SELECT count(*) AS n FROM accounts').get().n!==1)throw Error('account missing');if(db.prepare("SELECT value FROM settings WHERE key='jobs_enabled'").get().value!=='false')throw Error('jobs enabled');if(db.prepare('SELECT count(*) AS n FROM sessions').get().n!==0)throw Error('sessions retained');console.log('restored account, disabled jobs, cleared sessions');`);
  assert(fs.existsSync(`${backup}/data/snapshot/manifest.json`));
  console.log(JSON.stringify({ project, oldMigrations: Number(oldCount), newMigrations: Number(newCount), checks: [after, restoreCheck], backup }, null, 2));
} finally {
  compose('down', '-v', '--remove-orphans'); // Only this disposable project's volumes.
  // Retain the private fixture directory for inspecting recovery artifacts.
  console.log(`Disposable fixture artifacts: ${root}`);
}
