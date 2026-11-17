import { afterEach, expect, it, vi } from "vitest";
import { request } from "./api";
afterEach(() => vi.unstubAllGlobals());

it.each([502, 503, 504, 522])("explains a non-JSON %s response without exposing a proxy error page", async status => {
  vi.stubGlobal("fetch", vi.fn().mockImplementation(() => Promise.resolve(new Response("<h1>Private proxy detail</h1>", { status }))));
  await expect(request("/session")).rejects.toMatchObject({ status, code: "http_error", message: "Dakia is temporarily unavailable. Please try again." });
});
it("explains a failed connection without claiming authentication expired", async () => {
  vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));
  const failure = request("/session");
  await expect(failure).rejects.toBeInstanceOf(TypeError);
  await expect(failure).rejects.toMatchObject({ message: "Couldn’t connect to Dakia. Please try again." });
});
it("preserves cancellation instead of presenting a connection failure", async () => {
  const error = new DOMException("Cancelled", "AbortError");
  vi.stubGlobal("fetch", vi.fn().mockRejectedValue(error));
  await expect(request("/search")).rejects.toBe(error);
});
it("preserves confirmed authentication rejection", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ code: "unauthorized", message: "Sign in" }), { status: 401 })));
  await expect(request("/session")).rejects.toMatchObject({ status: 401, code: "unauthorized", message: "Sign in" });
});

it.each(["network", 502, 503, 504, 522] as const)("recovers a transient %s read failure", async failure => {
  const fetch = vi.fn();
  if (failure === "network") fetch.mockRejectedValueOnce(new TypeError("Failed to fetch"));
  else fetch.mockResolvedValueOnce(new Response("Unavailable", { status: failure }));
  fetch.mockResolvedValueOnce(Response.json({ ok: true }));
  vi.stubGlobal("fetch", fetch);
  await expect(request("/search")).resolves.toEqual({ ok: true });
  expect(fetch).toHaveBeenCalledTimes(2);
});

it.each(["POST", "PUT", "DELETE"])("never replays a %s after a lost acknowledgement or gateway failure", async method => {
  const fetch = vi.fn().mockRejectedValue(new TypeError("Failed to fetch"));
  vi.stubGlobal("fetch", fetch);
  await expect(request("/operations", { method })).rejects.toBeInstanceOf(TypeError);
  expect(fetch).toHaveBeenCalledTimes(1);
  fetch.mockClear().mockResolvedValue(new Response("Unavailable", { status: 502 }));
  await expect(request("/operations", { method })).rejects.toMatchObject({ status: 502 });
  expect(fetch).toHaveBeenCalledTimes(1);
});

it("does not retry a read cancelled during the retry delay", async () => {
  const controller = new AbortController();
  const fetch = vi.fn().mockImplementation(async () => {
    controller.abort();
    return new Response("Unavailable", { status: 503 });
  });
  vi.stubGlobal("fetch", fetch);
  await expect(request("/search", { signal: controller.signal })).rejects.toMatchObject({ name: "AbortError" });
  expect(fetch).toHaveBeenCalledTimes(1);
});
