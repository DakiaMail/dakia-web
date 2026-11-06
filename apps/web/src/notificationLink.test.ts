// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { notificationLink, rememberNotificationLink } from "./notificationLink";
afterEach(() => { vi.restoreAllMocks(); sessionStorage.clear(); history.replaceState(null, "", "/"); });
it("preserves an opaque notification target through sign-in, consumes once and prefers a newer click", () => {
  history.replaceState(null, "", "/?account=a&message=m%26one");
  rememberNotificationLink();
  expect(sessionStorage.getItem("dakia-notification-target")).not.toContain("subject");
  history.replaceState(null, "", "/");
  expect(notificationLink()).toEqual({ account: "a", message: "m&one" });
  expect(notificationLink()).toBeNull();
  history.replaceState(null, "", "/?account=a&message=older"); rememberNotificationLink();
  history.replaceState(null, "", "/?account=b&message=newer");
  expect(notificationLink()).toEqual({ account: "b", message: "newer" });
});
it("expires targets and tolerates malformed or unavailable tab storage", () => {
  history.replaceState(null, "", "/?account=a&message=m"); rememberNotificationLink();
  history.replaceState(null, "", "/");
  vi.spyOn(Date, "now").mockReturnValue(Date.now() + 600_001);
  expect(notificationLink()).toBeNull();
  sessionStorage.setItem("dakia-notification-target", "{"); expect(notificationLink()).toBeNull();
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw Error("blocked"); });
  history.replaceState(null, "", "/?account=a&message=m");
  expect(rememberNotificationLink).not.toThrow();
  expect(notificationLink()).toEqual({ account: "a", message: "m" });
});
