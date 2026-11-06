import { afterEach, expect, it, vi } from "vitest";
import { request } from "./api";
afterEach(() => vi.unstubAllGlobals());

it.each([502, 503, 504, 522])("explains a non-JSON %s response without exposing a proxy error page", async status => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("<h1>Private proxy detail</h1>", { status })));
  await expect(request("/session")).rejects.toMatchObject({ status, code: "http_error", message: "Can’t reach Dakia. Check your internet connection or VPN." });
});
it("explains a failed connection without claiming authentication expired", async () => {
  vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));
  const failure = request("/session");
  await expect(failure).rejects.toBeInstanceOf(TypeError);
  await expect(failure).rejects.toMatchObject({ message: "Can’t reach Dakia. Check your internet connection or VPN." });
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
