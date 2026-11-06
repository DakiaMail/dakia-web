// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, act } from "@testing-library/react";
import { InstallApp, InstallProvider } from "./InstallApp";

beforeEach(() => {
  vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() })));
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });
const renderApp = () => render(<InstallProvider><InstallApp /></InstallProvider>);
it("never prompts automatically, uses a captured offer once and handles dismissal", async () => {
  renderApp();
  const prompt = vi.fn(async () => {});
  const event = Object.assign(new Event("beforeinstallprompt", { cancelable: true }), { prompt, userChoice: Promise.resolve({ outcome: "dismissed" }) });
  act(() => { window.dispatchEvent(event); });
  expect(event.defaultPrevented).toBe(true);
  expect(prompt).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Install Dakia" }));
  await waitFor(() => expect(screen.queryByRole("button", { name: "Install Dakia" })).toBeNull());
  expect(prompt).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("status").textContent).toContain("install later");
});
it("does not claim completed installation from an accepted prompt", async () => {
  renderApp();
  act(() => { window.dispatchEvent(Object.assign(new Event("beforeinstallprompt"), { prompt: vi.fn(async () => {}), userChoice: Promise.resolve({ outcome: "accepted" }) })); });
  fireEvent.click(screen.getByRole("button", { name: "Install Dakia" }));
  await waitFor(() => expect(screen.getByRole("status").textContent).toContain("Finish installation"));
  act(() => { window.dispatchEvent(new Event("appinstalled")); });
  expect(screen.queryByRole("region", { name: "Install Dakia" })).toBeNull();
});
it("hides installation in standalone mode", () => {
  vi.stubGlobal("matchMedia", () => ({ matches: true, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
  renderApp();
  expect(screen.queryByRole("region", { name: "Install Dakia" })).toBeNull();
});
it("gives iPad Safari home-screen and notification instructions", () => {
  vi.spyOn(navigator, "platform", "get").mockReturnValue("MacIntel");
  Object.defineProperty(navigator, "maxTouchPoints", { value: 5, configurable: true });
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Mozilla/5.0 Version/18.0 Safari/605.1.15");
  renderApp();
  expect(screen.getByText(/In Safari, tap Share/)).toBeTruthy();
  expect(screen.getByText(/iOS 16.4/)).toBeTruthy();
});
it("handles failed prompts without retrying the spent event", async () => {
  renderApp();
  act(() => { window.dispatchEvent(Object.assign(new Event("beforeinstallprompt"), { prompt: vi.fn(async () => { throw new Error("unavailable"); }), userChoice: Promise.resolve({ outcome: "dismissed" }) })); });
  fireEvent.click(screen.getByRole("button", { name: "Install Dakia" }));
  await waitFor(() => expect(screen.getByRole("status").textContent).toContain("browser menu"));
  expect(screen.queryByRole("button", { name: "Install Dakia" })).toBeNull();
});
