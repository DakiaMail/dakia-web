import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AccountSetup } from "./AccountSetup";
import { request, api } from "./api";
vi.mock("./api", () => ({ request: vi.fn(), api: { discover: vi.fn() } }));
vi.mock("./ui", () => ({ Modal: ({ children }: any) => <div>{children}</div>, ErrorNotice: () => null }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });
function setup(enabled: boolean | "failed" = true) {
  if (enabled === "failed") vi.mocked(request).mockRejectedValue(new Error("offline"));
  else vi.mocked(request).mockResolvedValue({ enabled });
  render(<QueryClientProvider client={new QueryClient()}><AccountSetup onClose={() => {}} /></QueryClientProvider>);
}
it.each(["person@gmail.com", "Person@GOOGLEMAIL.COM"])("defaults %s to OAuth without discovery", async (email) => {
  setup();
  fireEvent.change(screen.getByLabelText("Email"), { target: { value: email } });
  await waitFor(() => expect(screen.queryByLabelText("Password")).toBeNull());
  expect(api.discover).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: /^Connect Gmail$/ })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Use app password instead" }));
  expect(screen.getByLabelText("App password")).toBeTruthy();
});
it("uses discovered Workspace Gmail without guessing other domains", async () => {
  setup();
  vi.mocked(api.discover).mockResolvedValue({ provider: "gmail", imap_host: "imap.gmail.com", imap_port: 993, imap_tls: "implicit", smtp_host: "smtp.gmail.com", smtp_port: 465, smtp_tls: "implicit", username: "user@company.test", source: "dns" });
  fireEvent.change(screen.getByLabelText("Email"), { target: { value: "user@company.test" } });
  expect(screen.getByLabelText("Password")).toBeTruthy();
  await waitFor(() => expect(screen.queryByLabelText("Password")).toBeNull());
  fireEvent.change(screen.getByLabelText("Email"), { target: { value: "other@example.test" } });
  expect(screen.getByLabelText("Password")).toBeTruthy();
});
it.each(["disabled", "failed"])("does not silently fall back to a password when Gmail status is %s", async (status) => {
  setup(status === "failed" ? "failed" : false);
  fireEvent.click(screen.getByRole("button", { name: /^Connect Gmail$/ }));
  await screen.findByRole("alert");
  expect(screen.queryByLabelText("App password")).toBeNull();
  expect((screen.getByRole("button", { name: /^Connect Gmail$/ }) as HTMLButtonElement).disabled).toBe(true);
});
it("restores OAuth default when the address changes", async () => {
  setup();
  fireEvent.change(screen.getByLabelText("Email"), { target: { value: "first@gmail.com" } });
  await screen.findByRole("button", { name: "Use app password instead" });
  fireEvent.click(screen.getByRole("button", { name: "Use app password instead" }));
  fireEvent.change(screen.getByLabelText("Email"), { target: { value: "second@gmail.com" } });
  expect(screen.queryByLabelText("App password")).toBeNull();
});
it("retries a failed status check without exposing a password", async () => {
  setup("failed");
  fireEvent.click(screen.getByRole("button", { name: /^Connect Gmail$/ }));
  await screen.findByRole("alert");
  vi.mocked(request).mockResolvedValue({ enabled: true });
  fireEvent.click(screen.getByRole("button", { name: "Try again" }));
  await waitFor(() => expect((screen.getByRole("button", { name: /^Connect Gmail$/ }) as HTMLButtonElement).disabled).toBe(false));
  expect(screen.queryByRole("alert")).toBeNull();
  expect(screen.queryByLabelText("App password")).toBeNull();
});
it.each(["user@gmail.com.evil.test", "user@notgmail.com", "user@example.test"])("does not guess Gmail for %s", email => {
  setup();
  fireEvent.change(screen.getByLabelText("Email"), { target: { value: email } });
  expect(screen.getByLabelText("Password")).toBeTruthy();
});
it("offers Gmail prominently before choosing a provider", async () => {
  setup();
  const button = await screen.findByRole("button", { name: /^Connect Gmail$/ });
  expect(button.querySelector('img[aria-hidden="true"]')).not.toBeNull();
  fireEvent.click(button);
  await waitFor(() => expect(screen.queryByLabelText("Password")).toBeNull());
  expect(screen.getByRole("button", { name: "Use app password instead" })).toBeTruthy();
});
