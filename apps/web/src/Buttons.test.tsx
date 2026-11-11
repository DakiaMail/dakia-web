import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { Button, ConfirmationButton, DakiaProvider, IconButton } from "@dakia/ui";

beforeEach(() => { vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }))); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.unstubAllGlobals(); });

it("keeps focus while pending, prevents duplicate actions, announces success and reverts", () => {
  vi.useFakeTimers();
  const onClick = vi.fn();
  const show = (status: "idle" | "pending" | "success" | "error") => <DakiaProvider><ConfirmationButton status={status} onClick={onClick}>Save settings</ConfirmationButton></DakiaProvider>;
  const view = render(show("idle"));
  const button = screen.getByRole("button", { name: "Save settings" });
  button.focus(); fireEvent.click(button);
  view.rerender(show("pending"));
  fireEvent.click(button);
  expect(onClick).toHaveBeenCalledTimes(1);
  expect(document.activeElement).toBe(button);
  expect(button.getAttribute("aria-busy")).toBe("true");
  view.rerender(show("success"));
  expect(screen.getByRole("button", { name: "Saved" })).toBe(button);
  expect(screen.getByRole("status").textContent).toBe("Saved");
  act(() => vi.advanceTimersByTime(1800));
  expect(screen.getByRole("button", { name: "Save settings" })).toBe(button);
  expect(document.activeElement).toBe(button);
  // A second request gets a fresh confirmation period.
  view.rerender(show("pending")); view.rerender(show("success"));
  expect(screen.getByRole("status").textContent).toBe("Saved");
  view.unmount();
  expect(vi.getTimerCount()).toBe(0);
});

it("doesn't confirm a failed action or allow a disabled action", () => {
  const onClick = vi.fn();
  render(<DakiaProvider><ConfirmationButton status="error" disabled onClick={onClick}>Save settings</ConfirmationButton></DakiaProvider>);
  fireEvent.click(screen.getByRole("button"));
  expect(onClick).not.toHaveBeenCalled();
  expect(screen.getByRole("status").textContent).toBe("");
});

it("defaults to non-submitting buttons and requires a named icon control", () => {
  const onSubmit = vi.fn();
  render(<DakiaProvider><form onSubmit={onSubmit}><Button>Cancel</Button><IconButton label="Refresh mail"><svg /></IconButton></form></DakiaProvider>);
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onSubmit).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "Refresh mail" }).getAttribute("type")).toBe("button");
});
