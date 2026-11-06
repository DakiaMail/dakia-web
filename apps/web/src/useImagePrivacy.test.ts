import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, renderHook } from "@testing-library/react";
import { useImagePrivacy } from "./useImagePrivacy";

afterEach(() => { cleanup(); vi.restoreAllMocks(); window.localStorage.clear(); });
it.each(["true", "false"])("preserves explicit legacy %s without rewriting storage", value => {
  window.localStorage.setItem("dakia:alice:always-load-images", value);
  const { result } = renderHook(() => useImagePrivacy("alice"));
  expect(result.current.alwaysLoadImages).toBe(value === "true");
  expect(window.localStorage.getItem("dakia:alice:always-load-images")).toBe(value);
  expect(result.current.blockHiddenImages).toBe(false);
});
it("hidden-image blocking defaults off and persists independently for each user", () => {
  const { result, rerender, unmount } = renderHook(({ user }) => useImagePrivacy(user), { initialProps: { user: "alice" } });
  expect(result.current.blockHiddenImages).toBe(false);
  act(() => result.current.setBlockHiddenImages(true));
  expect(result.current.alwaysLoadImages).toBe(true);
  rerender({ user: "bob" });
  expect(result.current.blockHiddenImages).toBe(false);
  rerender({ user: "alice" });
  expect(result.current.blockHiddenImages).toBe(true);
  unmount();
  const reopened = renderHook(() => useImagePrivacy("alice"));
  expect(reopened.result.current.blockHiddenImages).toBe(true);
  act(() => reopened.result.current.setBlockHiddenImages(false));
  expect(window.localStorage.getItem("dakia:alice:block-hidden-images")).toBe("false");
});
it("never requests images without a signed-in user", () => {
  const { result } = renderHook(() => useImagePrivacy());
  expect(result.current.alwaysLoadImages).toBe(false);
});
it("defaults to loading, persists both choices and isolates signed-in users", () => {
  const { result, rerender, unmount } = renderHook(({ user }) => useImagePrivacy(user), { initialProps: { user: "alice" } });
  expect(result.current.alwaysLoadImages).toBe(true);
  act(() => result.current.setAlwaysLoadImages(true));
  expect(result.current.alwaysLoadImages).toBe(true);
  rerender({ user: "bob" });
  expect(result.current.alwaysLoadImages).toBe(true);
  rerender({ user: "alice" });
  expect(result.current.alwaysLoadImages).toBe(true);
  act(() => result.current.setAlwaysLoadImages(false));
  unmount();
  const reopened = renderHook(() => useImagePrivacy("alice"));
  expect(reopened.result.current.alwaysLoadImages).toBe(false);
});
it("fails closed if browser storage is blocked and never opts in after failed save", () => {
  vi.spyOn(window.Storage.prototype, "getItem").mockImplementation(() => { throw new Error("blocked"); });
  vi.spyOn(window.Storage.prototype, "setItem").mockImplementation(() => { throw new Error("blocked"); });
  const { result } = renderHook(() => useImagePrivacy("alice"));
  expect(result.current.alwaysLoadImages).toBe(false);
  expect(() => act(() => result.current.setAlwaysLoadImages(true))).toThrow("blocked");
  expect(result.current.alwaysLoadImages).toBe(false);
});
it("revokes current image permission even when saving the disabled preference fails", () => {
  const { result } = renderHook(() => useImagePrivacy("alice"));
  act(() => result.current.setAlwaysLoadImages(true));
  vi.spyOn(window.Storage.prototype, "setItem").mockImplementation(() => { throw new Error("full"); });
  act(() => { expect(() => result.current.setAlwaysLoadImages(false)).toThrow("full"); });
  expect(result.current.alwaysLoadImages).toBe(false);
});
