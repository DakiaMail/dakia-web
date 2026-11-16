import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { DakiaProvider } from "@dakia/ui";
import { SearchSettings, searchSettingsApi } from "./SearchSettings";
beforeEach(() => { vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }))); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const policy = { mode: "automatic" as const, enabled: false, can_manage: true, resources: { memory_bytes: 2_000_000_000, cpus: 2 } };
function show(canManage = true) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity } } });
  client.setQueryData(["search-settings", "owner"], { ...policy, can_manage: canManage });
  render(<DakiaProvider><QueryClientProvider client={client}><SearchSettings userId="owner" /></QueryClientProvider></DakiaProvider>);
  return client;
}
it("saves the override and shows the effective state", async () => {
  const saved = { ...policy, mode: "on" as const, enabled: true };
  const save = vi.spyOn(searchSettingsApi, "save").mockResolvedValue(saved);
  const client = show();
  expect(screen.getByText("Search by meaning is off.")).toBeTruthy();
  fireEvent.click(screen.getByRole("radio", { name: /^On Enable/ }));
  fireEvent.click(screen.getByRole("button", { name: "Save search settings" }));
  await screen.findByText("Search by meaning is enabled.");
  expect(save).toHaveBeenCalledWith("on", expect.anything());
  expect(client.getQueryData(["search-settings", "owner"])).toEqual(saved); client.clear();
});
it("keeps the saved mode and draft visible when saving fails", async () => {
  vi.spyOn(searchSettingsApi, "save").mockRejectedValue(new Error("unavailable"));
  const client = show();
  fireEvent.click(screen.getByRole("radio", { name: /^On Enable/ }));
  fireEvent.click(screen.getByRole("button", { name: "Save search settings" }));
  await screen.findByRole("alert");
  expect(screen.getByText("Search by meaning is off.")).toBeTruthy();
  expect(client.getQueryData(["search-settings", "owner"])).toEqual(policy); client.clear();
});
it("shows non-administrators the current policy without editing controls", () => {
  const client = show(false);
  expect(screen.queryByRole("radio")).toBeNull();
  expect(screen.getByText(/Your administrator controls/)).toBeTruthy(); client.clear();
});
