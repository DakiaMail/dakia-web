import { ModelStore, TranslationService } from './service.mjs';
if (!process.env.DAKIA_MODEL_CACHE) throw new Error('DAKIA_MODEL_CACHE required');
const service = new TranslationService(new ModelStore(process.env.DAKIA_MODEL_CACHE));
const send = value => process.stdout.write(`${JSON.stringify(value)}\n`);
let buffer = ''; process.stdin.setEncoding('utf8');
for await (const chunk of process.stdin) {
  buffer += chunk;
  if (Buffer.byteLength(buffer) > 1_048_576) { send({ error: 'input_too_large' }); process.exitCode = 1; break; }
  let end;
  while ((end = buffer.indexOf('\n')) >= 0) {
    const line = buffer.slice(0, end); buffer = buffer.slice(end + 1);
    try {
      const request = JSON.parse(line);
      send({ result: await service.translate(request, progress => send({ job_id: request?.job?.id, progress })) });
    } catch { send({ error: 'translation_failed' }); }
  }
}
await service.terminate();
