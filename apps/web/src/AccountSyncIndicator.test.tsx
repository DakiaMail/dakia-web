// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AccountSyncIndicator } from "./AccountSyncIndicator";
import { api } from "./api";
import type { Account } from "@dakia/api-types";

afterEach(() => { cleanup(); vi.restoreAllMocks(); });
const account: Account = { id: "work", display_name: "Work", email: "work@example.invalid", state: "ready", provider: "imap", revision: 1 };
it("hides stale activity offline or after reconnect and isolates accounts", async () => {
  const read = vi.spyOn(api, "syncActive").mockResolvedValue(true);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const ui = (a: Account, online: boolean) => <QueryClientProvider client={client}><AccountSyncIndicator account={a} online={online} /></QueryClientProvider>;
  const view = render(ui(account, true));
  expect(await screen.findByRole("img", { name: "Syncing Work" })).toBeTruthy();
  view.rerender(ui(account, false));
  expect(screen.queryByRole("img")).toBeNull();
  view.rerender(ui({ ...account, state: "reconnect" }, true));
  expect(screen.queryByRole("img")).toBeNull();
  read.mockResolvedValue(false);
  view.rerender(ui({ ...account, id: "personal", display_name: "Personal" }, true));
  expect(screen.queryByRole("img")).toBeNull();
  expect(read).toHaveBeenCalledWith("work", expect.any(AbortSignal));
  client.clear();
});
it("failed activity reads never claim syncing", async () => {
  vi.spyOn(api, "syncActive").mockRejectedValue(new Error("unavailable"));
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(<QueryClientProvider client={client}><AccountSyncIndicator account={account} online /></QueryClientProvider>);
  await vi.waitFor(() => expect(client.getQueryState(["sync-active", account.id])?.status).toBe("error"));
  expect(screen.queryByRole("img")).toBeNull();
  client.clear();
});
