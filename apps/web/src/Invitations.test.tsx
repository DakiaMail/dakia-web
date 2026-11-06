import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Invitations } from "./Invitations";
import { api } from "./api";
vi.mock("./api", () => ({ api: { localAccess: vi.fn(), invitations: vi.fn(), createInvitation: vi.fn(), revokeInvitation: vi.fn() } }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });
function mount() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(<QueryClientProvider client={client}><Invitations userId="current-user" /></QueryClientProvider>);
}
it("does not expose invitation controls or request the list for a regular user", async () => {
  vi.mocked(api.localAccess).mockResolvedValue({ can_invite: false });
  mount(); await waitFor(() => expect(api.localAccess).toHaveBeenCalled());
  expect(screen.queryByRole("button", { name: "Create invitation" })).toBeNull();
  expect(api.invitations).not.toHaveBeenCalled();
});
it("creates a link for the owner and clears it when revoked", async () => {
  vi.mocked(api.localAccess).mockResolvedValue({ can_invite: true });
  vi.mocked(api.invitations).mockResolvedValue([]);
  vi.mocked(api.createInvitation).mockResolvedValue({ id: "invite-1", expires_at: 4102444800, url: "https://mail.test/auth/join#synthetic-invite" });
  vi.mocked(api.revokeInvitation).mockResolvedValue();
  mount(); fireEvent.click(await screen.findByRole("button", { name: "Create invitation" }));
  expect((await screen.findByLabelText("Invitation link") as HTMLInputElement).value).toBe("https://mail.test/auth/join#synthetic-invite");
  vi.mocked(api.invitations).mockResolvedValue([{ id: "invite-1", expires_at: 4102444800 }]);
  fireEvent.click(screen.getByRole("button", { name: "Create invitation" }));
  fireEvent.click(await screen.findByRole("button", { name: "Revoke invitation" }));
  await waitFor(() => expect(api.revokeInvitation).toHaveBeenCalledWith("invite-1", expect.anything()));
  await waitFor(() => expect(screen.queryByLabelText("Invitation link")).toBeNull());
});
