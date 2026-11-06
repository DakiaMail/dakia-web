// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AppUpdateProvider, useReloadGuard } from "./AppUpdate";
let version = "old";
let offline = false;
let hidden = false;
let fail = false;
let service: EventTarget & { register: ReturnType<typeof vi.fn> };
let registration: { update: ReturnType<typeof vi.fn>; waiting: null | { postMessage: ReturnType<typeof vi.fn> } };
const reload = vi.fn();
const flush = () => act(async () => { await vi.advanceTimersByTimeAsync(0); });
beforeEach(() => {
  vi.useFakeTimers(); version = "old"; offline = hidden = fail = false; reload.mockReset();
  vi.spyOn(navigator, "onLine", "get").mockImplementation(() => !offline);
  vi.spyOn(document, "visibilityState", "get").mockImplementation(() => hidden ? "hidden" : "visible");
  registration = { update: vi.fn().mockResolvedValue(undefined), waiting: null };
  service = Object.assign(new EventTarget(), { register: vi.fn().mockResolvedValue(registration) });
  Object.defineProperty(navigator, "serviceWorker", { configurable: true, value: service });
  vi.stubGlobal("fetch", vi.fn(async () => { if (fail) throw new TypeError("offline"); return new Response(JSON.stringify({ version })); }));
});
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); Reflect.deleteProperty(navigator, "serviceWorker"); });
const mount = (child = <span>Open mail</span>) => render(<AppUpdateProvider enabled buildId="old" reload={reload}>{child}</AppUpdateProvider>);
it("does not announce initial installation or reload when another tab activates a worker", async () => {
  mount(); await flush(); service.dispatchEvent(new Event("controllerchange"));
  expect(screen.queryByRole("complementary", { name: "App update" })).toBeNull(); expect(reload).not.toHaveBeenCalled();
  expect(service.register).toHaveBeenCalledWith("/sw.js", { updateViaCache: "none" });
});
it("offers a new build, waits for consent, and dismisses only that version", async () => {
  version = "new"; mount(); await flush();
  expect(screen.getByText("New version available")).toBeTruthy(); expect(reload).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Later" }));
  await act(async () => { await vi.advanceTimersByTimeAsync(300_000); });
  expect(screen.queryByText("New version available")).toBeNull();
  version = "newer"; await act(async () => { await vi.advanceTimersByTimeAsync(300_000); });
  expect(screen.getByText("New version available")).toBeTruthy();
});
it("coalesces resume events and skips hidden or offline polling", async () => {
  mount(); await flush();
  for (let i = 0; i < 30; i++) window.dispatchEvent(new Event("visibilitychange"));
  await flush(); expect(fetch).toHaveBeenCalledTimes(1);
  hidden = true; await act(async () => { await vi.advanceTimersByTimeAsync(300_000); });
  expect(fetch).toHaveBeenCalledTimes(1);
  hidden = false; offline = true; window.dispatchEvent(new Event("online")); await flush();
  expect(fetch).toHaveBeenCalledTimes(1);
  offline = false; window.dispatchEvent(new Event("online")); await flush();
  expect(fetch).toHaveBeenCalledTimes(2);
});
it("background failures remain quiet and an explicit reload failure preserves the app", async () => {
  fail = true; mount(); await flush(); expect(screen.queryByRole("complementary")).toBeNull();
  fail = false; version = "new"; await act(async () => { await vi.advanceTimersByTimeAsync(300_000); });
  fail = true; fireEvent.click(screen.getByRole("button", { name: "Reload" })); await flush();
  expect(screen.getByText(/Couldn’t load the update/)).toBeTruthy(); expect(screen.getByText("Open mail")).toBeTruthy(); expect(reload).not.toHaveBeenCalled();
});
function DialogGuard() { useReloadGuard(); return <div>Unfinished draft</div>; }
it("does not reload while a dialog is open", async () => {
  version = "new"; const view = mount(<DialogGuard />); await flush();
  expect((screen.getByRole("button", { name: "Reload" }) as HTMLButtonElement).disabled).toBe(true);
  view.rerender(<AppUpdateProvider enabled buildId="old" reload={reload}><span>Open mail</span></AppUpdateProvider>); await flush();
  fireEvent.click(screen.getByRole("button", { name: "Reload" })); await flush(); expect(reload).toHaveBeenCalledTimes(1);
});
it("activates a waiting worker only on consent and reloads once after activation", async () => {
  version = "new"; registration.waiting = { postMessage: vi.fn() }; mount(); await flush();
  expect(registration.waiting.postMessage).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Reload" })); await flush();
  expect(registration.waiting.postMessage).toHaveBeenCalledWith({ type: "DAKIA_SKIP_WAITING" }); expect(reload).not.toHaveBeenCalled();
  service.dispatchEvent(new Event("controllerchange")); await flush(); service.dispatchEvent(new Event("controllerchange"));
  expect(reload).toHaveBeenCalledTimes(1);
});
it("a worker activation timeout offers retry without discarding the current app", async () => {
  version = "new"; registration.waiting = { postMessage: vi.fn() }; mount(); await flush();
  fireEvent.click(screen.getByRole("button", { name: "Reload" })); await flush();
  await act(async () => { await vi.advanceTimersByTimeAsync(5001); });
  expect(screen.getByText(/Couldn’t load the update/)).toBeTruthy(); expect(reload).not.toHaveBeenCalled();
});
