import registry from "../../../workers/translation/models.json";

export type TranslationMode = "host" | "device" | "automatic";
export type TranslationPolicy = { mode: TranslationMode; can_manage: boolean };
export type DownloadProgress = { stage: "downloading" | "preparing" | "translating"; downloaded: number; total: number };
type Artifact = readonly [string, number, string];
function spec(values: (string | number)[]): Artifact {
  const [url, size, hash] = values;
  if (typeof url !== "string" || typeof size !== "number" || typeof hash !== "string" || !/^[a-f0-9]{64}$/.test(hash)) throw new Error("Invalid translation registry");
  return [url, size, hash];
}
const models = registry.models.map(m => ({ ...m, files: { model: spec(m.files.model), shortlist: spec(m.files.shortlist), vocabs: m.files.vocabs.map(spec) } }));
const CACHE = "dakia-translation-packs-v1";
const wasmUrl = new URL("../../../workers/translation/browser/bergamot-translator.wasm", import.meta.url).href;
const wasmHash = "f38ef807636a7c994afedaab7ff8ffe0d590d21897f05139f747b80fd7bbe926";
const digest = async (bytes: ArrayBuffer) => [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))].map(x => x.toString(16).padStart(2, "0")).join("");
export class DeviceTranslationError extends Error {
  constructor(message: string, public readonly fallback = true) { super(message); }
}
export type SavedPack = { source: string; name: string; bytes: number };
export async function savedPacks(): Promise<SavedPack[]> {
  if (!("caches" in globalThis)) return [];
  const cache = await caches.open(CACHE);
  const keys = new Set((await cache.keys()).map(r => new URL(r.url).pathname));
  return models.filter(m => [m.files.model, m.files.shortlist, ...m.files.vocabs].every(a => keys.has(`/api/translation/models/${a[2]}`)))
    .map(m => ({ source: m.source, name: m.sourceName, bytes: [m.files.model, m.files.shortlist, ...m.files.vocabs].reduce((n,a) => n + a[1], 0) }));
}
export async function removePack(source: string) {
  const m = models.find(m => m.source === source);
  if (!m) return;
  deviceTranslator.reset();
  const cache = await caches.open(CACHE);
  const keys = new Set((await cache.keys()).map(r => new URL(r.url).pathname));
  const files = (model: typeof m) => [model.files.model, model.files.shortlist, ...model.files.vocabs];
  const retained = new Set(models.filter(other => other.source !== source && files(other).every(a => keys.has(`/api/translation/models/${a[2]}`))).flatMap(other => files(other).map(a => a[2])));
  for (const a of files(m)) if (!retained.has(a[2])) await cache.delete(`/api/translation/models/${a[2]}`);
}
export const formatBytes = (n: number) => `${(n / 1_000_000).toFixed(1)} MB`;

class DeviceTranslator {
  private worker: Worker | null = null;
  private source = "";
  private owner = "";
  private busy = false;
  private operation: AbortController | null = null;
  private serial = 0;
  private idle?: ReturnType<typeof setTimeout>;
  private pending = new Map<number, { resolve: (x: unknown) => void; reject: (e: Error) => void; timer: ReturnType<typeof setTimeout> }>();
  private stopWorker() {
    clearTimeout(this.idle);
    this.worker?.terminate(); this.worker = null; this.source = "";
    for (const p of this.pending.values()) { clearTimeout(p.timer); p.reject(new DOMException("Translation cancelled", "AbortError")); }
    this.pending.clear();
  }
  reset() { this.operation?.abort(); this.stopWorker(); }
  releaseOwner(owner: string) { if (this.owner === owner) this.reset(); }
  private call(name: string, args: unknown[], transfer: Transferable[] = []): Promise<unknown> {
    clearTimeout(this.idle);
    if (!this.worker) return Promise.reject(new DeviceTranslationError("Translation is not ready on this device."));
    return new Promise((resolve, reject) => {
      const id = ++this.serial;
      const timer = setTimeout(() => { this.pending.delete(id); reject(new DeviceTranslationError("This device took too long to translate the message.")); this.stopWorker(); }, 90_000);
      this.pending.set(id, { resolve, reject, timer });
      try { this.worker!.postMessage({ id, name, args }, transfer); }
      catch { clearTimeout(timer); this.pending.delete(id); reject(new DeviceTranslationError("This device could not start translation.")); }
    });
  }
  private start() {
    this.stopWorker();
    const worker = new Worker(new URL("./translation.worker.js", import.meta.url), { type: "module" });
    this.worker = worker;
    const fail = () => {
      if (this.worker !== worker) return;
      for (const p of this.pending.values()) { clearTimeout(p.timer); p.reject(new DeviceTranslationError("Translation is not available on this device.")); }
      this.pending.clear(); this.stopWorker();
    };
    worker.onmessage = ({ data }) => {
      if (this.worker !== worker) return;
      if (data.fatal) { fail(); return; }
      const p = this.pending.get(data.id); if (!p) return;
      clearTimeout(p.timer); this.pending.delete(data.id);
      if (!this.pending.size) this.idle = setTimeout(() => this.reset(), 15_000);
      data.error ? p.reject(new DeviceTranslationError("This device could not finish the translation.")) : p.resolve(data.result);
    };
    worker.onerror = fail;
  }

  async translate(owner: string, source: string, text: string, html: boolean, outerSignal: AbortSignal, progress: (p: DownloadProgress) => void): Promise<string> {
    if (this.busy) throw new DeviceTranslationError("Another message is being translated. Try again in a moment.", false);
    if (!globalThis.isSecureContext || typeof globalThis.Worker !== "function" || !("caches" in globalThis) || !globalThis.crypto?.subtle || !("WebAssembly" in globalThis) || !("Segmenter" in Intl)) {
      throw new DeviceTranslationError("Translation is not supported on this device.");
    }
    if (new TextEncoder().encode(text).byteLength > 256_000) throw new DeviceTranslationError("This message is too large to translate on this device.");
    const m = models.find(m => m.source === source && m.target === "en");
    if (!m) throw new DeviceTranslationError("This language is not available for translation.", false);
    outerSignal.throwIfAborted(); this.busy = true; this.owner = owner;
    const operation = new AbortController(); this.operation = operation;
    const signal = operation.signal;
    const abort = () => { operation.abort(); this.stopWorker(); };
    outerSignal.addEventListener("abort", abort, { once: true });
    try {
      if (!this.worker || this.source !== source) {
        this.stopWorker();
        const files = [m.files.model, m.files.shortlist, ...m.files.vocabs];
        const total = files.reduce((n,a) => n+a[1],0) + 4_960_506;
        let downloaded = 0;
        let lastReport = -Infinity;
        const report = (partial = 0) => {
          const now = performance.now();
          if (downloaded + partial === total || now - lastReport >= 80) {
            lastReport = now; progress({ stage: "downloading", downloaded: downloaded + partial, total });
          }
        };
        const cache = await caches.open(CACHE);
        const artifact = async (url: string, size: number, hash: string) => {
          signal.throwIfAborted();
          const cached = await cache.match(url);
          if (cached) {
            const bytes = await cached.arrayBuffer();
            if (bytes.byteLength === size && await digest(bytes) === hash) { downloaded += size; report(); return bytes; }
            await cache.delete(url);
          }
          report();
          const response = await fetch(url, { signal, credentials: "same-origin", cache: "no-store", redirect: "error" });
          if (!response.ok || !response.body) throw new DeviceTranslationError("The language download failed. Try again.");
          const reader = response.body.getReader(), bytes = new Uint8Array(size);
          let received = 0;
          try {
            while (true) {
              const { value, done } = await reader.read(); if (done) break;
              if (received + value.byteLength > size) throw new DeviceTranslationError("The language download could not be verified. Try again.");
              bytes.set(value, received);
              received += value.byteLength; report(received);
            }
          } catch (error) { await reader.cancel().catch(() => {}); throw error; }
          finally { reader.releaseLock(); }
          if (received !== size || await digest(bytes.buffer) !== hash) throw new DeviceTranslationError("The language download could not be verified. Try again.");
          signal.throwIfAborted();
          try { await cache.put(url, new Response(bytes)); }
          catch { throw new DeviceTranslationError("There is not enough browser storage to save this language. Remove a saved language or free some space."); }
          downloaded += size; report(); return bytes.buffer;
        };
        const wasmBinary = await artifact(wasmUrl, 4_960_506, wasmHash);
        const buffers: ArrayBuffer[] = [];
        for (const a of files) buffers.push(await artifact(`/api/translation/models/${a[2]}`, a[1], a[2]));
        signal.throwIfAborted(); progress({ stage: "preparing", downloaded: total, total });
        this.start();
        await this.call("initialize", [{ wasmBinary }], [wasmBinary]);
        signal.throwIfAborted();
        await this.call("loadTranslationModel", [{ from: source, to: "en" }, { model: buffers[0], shortlist: buffers[1], vocabs: buffers.slice(2), config: m.config }], buffers);
        this.source = source;
      }
      signal.throwIfAborted(); progress({ stage: "translating", downloaded: 0, total: 0 });
      const result = await this.call("translate", [{ texts: [{ text, html }] }]) as { target: { text: string } }[];
      signal.throwIfAborted();
      if (typeof result?.[0]?.target?.text !== "string") throw new DeviceTranslationError("The translation could not be read. Try again.");
      return result[0].target.text;
    } catch (error) {
      this.stopWorker(); signal.throwIfAborted();
      if (error instanceof DeviceTranslationError) throw error;
      throw new DeviceTranslationError("Translation could not finish on this device. Try again.");
    } finally { outerSignal.removeEventListener("abort", abort); if (this.operation === operation) this.operation = null; this.busy = false; }
  }
}
export const deviceTranslator = new DeviceTranslator();
