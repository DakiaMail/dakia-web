// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { NotificationSettings } from "./NotificationSettings";
import { extraApi } from "./extra-api";
import { ApiFailure } from "./api";
const subscribe = vi.fn(), unsubscribe = vi.fn(), showNotification = vi.fn(), permission = vi.fn();
let active = true, registered = false;
const subscription = { endpoint: "https://fcm.googleapis.com/synthetic", toJSON: () => ({ endpoint: "synthetic" }), unsubscribe };
beforeEach(() => {
  active = true; registered = false;
  vi.stubGlobal("crypto", { subtle: { digest: vi.fn(async () => new Uint8Array([1, 2, 255]).buffer) } });
  vi.stubGlobal("PushManager", function () {});
  vi.stubGlobal("Notification", { permission: "default", requestPermission: permission.mockResolvedValue("granted") });
  Object.defineProperty(navigator, "serviceWorker", { configurable: true, value: { getRegistration: vi.fn(async () => ({ active: active ? {} : null, pushManager: { getSubscription: async () => registered ? subscription : null, subscribe: subscribe.mockResolvedValue(subscription) }, showNotification })) } });
  vi.spyOn(extraApi, "pushKey").mockResolvedValue("AQID");
  vi.spyOn(extraApi, "pushStatus").mockResolvedValue(true);
  vi.spyOn(extraApi, "subscribe").mockResolvedValue();
  vi.spyOn(extraApi, "disablePush").mockResolvedValue();
  unsubscribe.mockResolvedValue(true); showNotification.mockResolvedValue(undefined);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.clearAllMocks(); Reflect.deleteProperty(navigator, "serviceWorker"); });
it("checks configuration without asking permission, enables, tests locally and removes the server subscription before unsubscribing", async () => {
  render(<NotificationSettings />);
  await waitFor(() => expect(screen.getByRole("button", { name: "Enable notifications" }).hasAttribute("disabled")).toBe(false));
  expect(permission).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Enable notifications" }));
  await screen.findByRole("button", { name: "Disable notifications" });
  expect(extraApi.subscribe).toHaveBeenCalledWith({ endpoint: "synthetic" });
  fireEvent.click(screen.getByRole("button", { name: "Test notification" }));
  await screen.findByText(/This checks this device/); expect(showNotification).toHaveBeenCalledOnce();
  registered = true;
  fireEvent.click(screen.getByRole("button", { name: "Disable notifications" }));
  await screen.findByRole("button", { name: "Enable notifications" });
  expect(extraApi.disablePush).toHaveBeenCalledWith("0102ff");
  expect(vi.mocked(extraApi.disablePush).mock.invocationCallOrder[0]).toBeLessThan(unsubscribe.mock.invocationCallOrder[0]);
});
it("restores status from browser and server, including blocked permission", async () => {
  registered = true; Object.defineProperty(Notification, "permission", { value: "denied", configurable: true });
  render(<NotificationSettings />);
  await screen.findByText("Blocked in your browser or device settings.");
  await waitFor(() => expect(screen.getByRole("button", { name: "Disable notifications" }).hasAttribute("disabled")).toBe(false));
  expect(screen.queryByRole("button", { name: "Test notification" })).toBeNull();
});
it("keeps alerts enabled if server removal fails", async () => {
  registered = true; vi.mocked(extraApi.disablePush).mockRejectedValue(new Error("Disconnected"));
  render(<NotificationSettings />);
  fireEvent.click(await screen.findByRole("button", { name: "Disable notifications" }));
  await screen.findByRole("alert"); expect(unsubscribe).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "Disable notifications" })).toBeTruthy();
});
it("explains missing server setup before asking permission", async () => {
  vi.mocked(extraApi.pushKey).mockRejectedValue(new ApiFailure(503, "push_not_configured", "Unavailable"));
  render(<NotificationSettings />);
  await screen.findByText(/aren’t set up on this Dakia server/); expect(permission).not.toHaveBeenCalled();
});
it("fails promptly when there is no active worker", async () => {
  active = false; render(<NotificationSettings />);
  await waitFor(() => expect(screen.getByRole("button", { name: "Enable notifications" }).hasAttribute("disabled")).toBe(false));
  fireEvent.click(screen.getByRole("button", { name: "Enable notifications" }));
  await screen.findByText("Reload Dakia, then try again."); expect(extraApi.subscribe).not.toHaveBeenCalled();
});

it("can disable an existing subscription even when push configuration has been removed", async () => {
  registered = true;
  vi.mocked(extraApi.pushKey).mockRejectedValue(new ApiFailure(503, "push_not_configured", "Unavailable"));
  render(<NotificationSettings />);
  const button = await screen.findByRole("button", { name: "Disable notifications" });
  await waitFor(() => expect(button.hasAttribute("disabled")).toBe(false));
  fireEvent.click(button);
  await screen.findByRole("button", { name: "Enable notifications" });
  expect(extraApi.disablePush).toHaveBeenCalledOnce();
});
