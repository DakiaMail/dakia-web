// @vitest-environment node
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

vi.mock("../../../workers/translation/models.json", async () => {
  const { createHash } = await import("node:crypto");
  const artifact = (text: string) => ["https://example.invalid/" + text, text.length, createHash("sha256").update(text).digest("hex")];
  return { default: { models: [{ source: "et", sourceName: "Estonian", target: "en", config: {}, files: { model: artifact("model"), shortlist: artifact("shortlist"), vocabs: [artifact("vocab")] } }] } };
});

const wasm = new Uint8Array(readFileSync(new URL("../../../workers/translation/browser/bergamot-translator.wasm", import.meta.url)));
const hash = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const files = new Map(["model", "shortlist", "vocab"].map(text => {
  const bytes = new TextEncoder().encode(text);
  return [hash(bytes), bytes];
}));
const never = <T>() => new Promise<T>(() => {});
const buffer = (bytes: Uint8Array) => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer;
const bytesFor = (url: string) => files.get(url.split("/").at(-1)!) || wasm;
const response = (bytes: Uint8Array) => new Response(bytes as BodyInit);
const outcome = <T>(promise: Promise<T>) => promise.then(value => ({ value, error: undefined }), error => ({ value: undefined, error }));

class WorkerFixture {
  static instances: WorkerFixture[] = [];
  onmessage?: (event: { data: unknown }) => void;
  onerror?: () => void;
  terminate = vi.fn();
  constructor() { WorkerFixture.instances.push(this); }
  postMessage({ id, name }: { id: number; name: string }) {
    queueMicrotask(() => this.onmessage?.({ data: { id, result: name === "translate" ? [{ target: { text: "Translated fixture" } }] : null } }));
  }
}

let device: typeof import("./deviceTranslation");
let cache: { match: ReturnType<typeof vi.fn>; delete: ReturnType<typeof vi.fn>; put: ReturnType<typeof vi.fn> };
let open: ReturnType<typeof vi.fn>;
let fetcher: ReturnType<typeof vi.fn>;
beforeEach(async () => {
  vi.resetModules();
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date", "performance"] });
  WorkerFixture.instances = [];
  cache = {
    match: vi.fn(async (url: string) => ({ arrayBuffer: async () => buffer(bytesFor(url)) })),
    delete: vi.fn(async () => true), put: vi.fn(async () => {}),
  };
  open = vi.fn(async () => cache);
  fetcher = vi.fn(async (url: string) => response(bytesFor(url)));
  vi.stubGlobal("isSecureContext", true);
  vi.stubGlobal("Worker", WorkerFixture);
  vi.stubGlobal("caches", { open });
  vi.stubGlobal("fetch", fetcher);
  vi.stubGlobal("crypto", { subtle: { digest: async (_algorithm: string, bytes: ArrayBuffer) => buffer(new Uint8Array(createHash("sha256").update(new Uint8Array(bytes)).digest())) } });
  device = await import("./deviceTranslation");
});
afterEach(() => {
  device?.deviceTranslator.reset();
  vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks();
});
const translate = (signal = new AbortController().signal) => outcome(device.deviceTranslator.translate("fixture-owner", "et", "Synthetic text", false, signal, () => {}));
const expectFallback = (error: unknown) => {
  expect(error).toBeInstanceOf(device.DeviceTranslationError);
  expect((error as InstanceType<typeof device.DeviceTranslationError>).fallback).toBe(true);
};

it.each(["open", "match", "read", "write"])("bounds a stalled cache %s without starting a late worker", async stage => {
  let finish!: (value: unknown) => void;
  const stalled = new Promise(resolve => { finish = resolve; });
  if (stage === "open") open.mockReturnValue(stalled);
  if (stage === "match") cache.match.mockReturnValueOnce(stalled);
  if (stage === "read") cache.match.mockResolvedValueOnce({ arrayBuffer: () => stalled });
  if (stage === "write") { cache.match.mockResolvedValueOnce(undefined); cache.put.mockReturnValueOnce(stalled); }
  const result = translate();
  await vi.advanceTimersByTimeAsync(10_000);
  expectFallback((await result).error);
  expect(WorkerFixture.instances).toHaveLength(0);
  finish(stage === "open" ? cache : stage === "match" ? { arrayBuffer: async () => buffer(wasm) } : stage === "read" ? buffer(wasm) : undefined);
  await vi.advanceTimersByTimeAsync(0);
  expect(WorkerFixture.instances).toHaveLength(0);
  expect(vi.getTimerCount()).toBe(0);
});

it("bounds a stalled first response, aborts it, and ignores late success", async () => {
  cache.match.mockResolvedValueOnce(undefined);
  let finish!: (value: Response) => void;
  fetcher.mockReturnValueOnce(new Promise<Response>(resolve => { finish = resolve; }));
  const result = translate();
  await vi.advanceTimersByTimeAsync(150_000);
  expectFallback((await result).error);
  expect(fetcher.mock.calls[0][1].signal.aborted).toBe(true);
  finish(response(wasm));
  await vi.advanceTimersByTimeAsync(0);
  expect(cache.put).not.toHaveBeenCalled();
  expect(WorkerFixture.instances).toHaveLength(0);
  expect(vi.getTimerCount()).toBe(0);
});

it.each([false, true])("bounds a stalled stream and ignores empty keepalives=%s", async emptyKeepalives => {
  cache.match.mockResolvedValueOnce(undefined);
  const cancel = vi.fn(() => never<void>()), releaseLock = vi.fn();
  const read = vi.fn(() => emptyKeepalives
    ? new Promise(resolve => setTimeout(() => resolve({ done: false, value: new Uint8Array() }), 20_000))
    : never());
  fetcher.mockResolvedValueOnce({ ok: true, body: { getReader: () => ({ read, cancel, releaseLock }) } });
  const result = translate();
  await vi.advanceTimersByTimeAsync(30_000);
  expectFallback((await result).error);
  expect(cancel).toHaveBeenCalledOnce();
  expect(releaseLock).toHaveBeenCalledOnce();
  expect(cache.put).not.toHaveBeenCalled();
  expect(WorkerFixture.instances).toHaveLength(0);
});

it.each(["before", "cache", "stream"])("preserves explicit cancellation %s without fallback", async stage => {
  const abort = new AbortController();
  if (stage === "before") abort.abort();
  if (stage === "cache") open.mockReturnValue(never());
  if (stage === "stream") {
    cache.match.mockResolvedValueOnce(undefined);
    fetcher.mockResolvedValueOnce({ ok: true, body: { getReader: () => ({ read: () => never(), cancel: async () => {}, releaseLock() {} }) } });
  }
  const result = translate(abort.signal);
  await vi.advanceTimersByTimeAsync(0); abort.abort();
  expect((await result).error?.name).toBe("AbortError");
  expect(cache.put).not.toHaveBeenCalled();
  expect(WorkerFixture.instances).toHaveLength(0);
  expect(vi.getTimerCount()).toBe(0);
});

it("allows a progressing download longer than the first-response budget", async () => {
  cache.match.mockResolvedValueOnce(undefined);
  let chunk = 0;
  const read = () => chunk === 6 ? Promise.resolve({ done: true, value: undefined })
    : new Promise(resolve => setTimeout(() => {
      const start = Math.floor(wasm.length * chunk / 6), end = Math.floor(wasm.length * ++chunk / 6);
      resolve({ done: false, value: wasm.slice(start, end) });
    }, 29_000));
  fetcher.mockResolvedValueOnce({ ok: true, body: { getReader: () => ({ read, cancel: async () => {}, releaseLock() {} }) } });
  const result = translate();
  await vi.advanceTimersByTimeAsync(174_000);
  expect((await result).value).toBe("Translated fixture");
  expect(cache.put).toHaveBeenCalledOnce();
  expect(cache.delete).not.toHaveBeenCalled();
  expect(WorkerFixture.instances).toHaveLength(1);
});

it("rejects unverified bytes without deleting other completed artifacts", async () => {
  const modelHash = hash(new TextEncoder().encode("model"));
  cache.match.mockImplementation(async (url: string) => url.endsWith(modelHash) ? undefined : { arrayBuffer: async () => buffer(bytesFor(url)) });
  fetcher.mockResolvedValueOnce(response(new TextEncoder().encode("bogus")));
  const result = translate();
  await vi.advanceTimersByTimeAsync(0);
  expectFallback((await result).error);
  expect(cache.put).not.toHaveBeenCalled();
  expect(cache.delete).not.toHaveBeenCalled();
  expect(WorkerFixture.instances).toHaveLength(0);
});
