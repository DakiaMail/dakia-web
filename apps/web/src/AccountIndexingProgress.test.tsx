// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AccountIndexingProgress } from "./AccountIndexingProgress";
import { DakiaProvider } from "@dakia/ui";
import { api } from "./api";
import type { Account, IndexingStatus } from "@dakia/api-types";

const account: Account = { id: "work", display_name: "Work", email: "work@example.invalid", state: "ready", provider: "imap", revision: 1 };
const initial: IndexingStatus = { account_id: "work", state: "discovering", discovery_complete: false, eligible: 10, processed: 2, backlog: 8, gaps: 0 };
beforeEach(() => { vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }))); vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} }); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.useRealTimers(); });

it("waits before showing work, distinguishes discovery, and disappears without shifting focus", async () => {
  vi.useFakeTimers();
  vi.spyOn(api, "indexingStatus").mockResolvedValue(initial);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(<DakiaProvider><QueryClientProvider client={client}><AccountIndexingProgress account={account} online><button>Work account</button></AccountIndexingProgress></QueryClientProvider></DakiaProvider>);
  const button = screen.getByRole("button");button.focus();
  await act(async () => { await vi.advanceTimersByTimeAsync(100); });
  expect(screen.queryByRole("progressbar")).toBeNull();
  await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
  const bar = screen.getByRole("progressbar", { name: "Indexing Work" });
  expect(bar.getAttribute("aria-valuenow")).toBeNull();
  expect(bar.getAttribute("tabindex")).toBeNull();
  act(() => client.setQueryData(["indexing", account.id], { ...initial, state: "indexing", discovery_complete: true }));
  await act(async () => { await vi.advanceTimersByTimeAsync(1); });
  expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("20");
  act(() => client.setQueryData(["indexing", account.id], { ...initial, state: "error", discovery_complete: true }));
  await act(async () => { await vi.advanceTimersByTimeAsync(1); });
  expect(screen.getByRole("progressbar").getAttribute("aria-valuetext")).toBe("Indexing will retry");
  expect(document.activeElement).toBe(button);
  act(() => client.setQueryData(["indexing", account.id], { ...initial, state: "complete", processed: 10, backlog: 0, discovery_complete: true }));
  await act(async () => { await vi.advanceTimersByTimeAsync(1); });
  expect(screen.queryByRole("progressbar")).toBeNull();
  expect(document.activeElement).toBe(button);
  client.clear();
});

it("suppresses quick updates and preserves paused progress offline", async () => {
  vi.useFakeTimers();
  vi.spyOn(api,"indexingStatus").mockResolvedValue({ ...initial, state: "indexing", discovery_complete: true });
  const client = new QueryClient();
  const ui = (online: boolean) => <DakiaProvider><QueryClientProvider client={client}><AccountIndexingProgress account={account} online={online}><button>Work account</button></AccountIndexingProgress></QueryClientProvider></DakiaProvider>;
  const view=render(ui(true));
  await act(async () => { await vi.advanceTimersByTimeAsync(100); });
  act(() => client.setQueryData(["indexing", account.id], { ...initial, state: "complete" }));
  await act(async () => { await vi.advanceTimersByTimeAsync(2100); });
  expect(screen.queryByRole("progressbar")).toBeNull();
  act(() => client.setQueryData(["indexing", account.id], { ...initial, state: "indexing", discovery_complete: true }));
  await act(async () => { await vi.advanceTimersByTimeAsync(2100); });
  view.rerender(ui(false));
  expect(screen.getByRole("progressbar").getAttribute("aria-valuetext")).toBe("Indexing paused");
  client.clear();
});

it("keeps polling local progress through a provider reconnect and hides completed work", async () => {
  vi.useFakeTimers();
  const poll=vi.spyOn(api,"indexingStatus").mockResolvedValue({...initial,state:"indexing",discovery_complete:true});
  const client=new QueryClient();
  render(<DakiaProvider><QueryClientProvider client={client}><AccountIndexingProgress account={{...account,state:"reconnect"}} online><button>Work account</button></AccountIndexingProgress></QueryClientProvider></DakiaProvider>);
  const button=screen.getByRole("button");button.focus();
  await act(async()=>{await vi.advanceTimersByTimeAsync(2100);});
  expect(screen.getByRole("progressbar").getAttribute("aria-valuetext")).toBe("20% indexed");
  poll.mockResolvedValue({...initial,state:"indexing",discovery_complete:true,processed:10,backlog:1});
  await act(async()=>{await vi.advanceTimersByTimeAsync(5100);});
  expect(poll).toHaveBeenCalledTimes(2);
  expect(screen.getByRole("progressbar").getAttribute("aria-valuetext")).toBe("Finishing search");
  expect(document.activeElement).toBe(button);
  poll.mockResolvedValue({...initial,state:"complete",discovery_complete:true,processed:10,backlog:0});
  await act(async()=>{await vi.advanceTimersByTimeAsync(5100);});
  expect(poll).toHaveBeenCalledTimes(3);
  expect(screen.queryByRole("progressbar")).toBeNull();
  expect(document.activeElement).toBe(button);
  await act(async()=>{await vi.advanceTimersByTimeAsync(15000);});
  expect(poll).toHaveBeenCalledTimes(3);
  client.clear();
});
