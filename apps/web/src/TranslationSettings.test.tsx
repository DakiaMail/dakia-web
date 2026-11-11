import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { DakiaProvider } from "@dakia/ui";
import { TranslationSettings } from "./TranslationSettings";
import { translationPolicyKey, translationSettingsApi } from "./translationPolicy";
import * as device from "./deviceTranslation";
beforeEach(() => { vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }))); });
afterEach(()=>{cleanup();vi.restoreAllMocks();vi.unstubAllGlobals();});
function show(canManage: boolean) {
  const client=new QueryClient({defaultOptions:{queries:{retry:false,staleTime:Infinity}}});
  client.setQueryData(translationPolicyKey("owner"),{mode:"host",can_manage:canManage});
  vi.spyOn(translationSettingsApi,"get").mockResolvedValue({mode:"host",can_manage:canManage});
  vi.spyOn(device,"savedPacks").mockResolvedValue([]);
  render(<DakiaProvider><QueryClientProvider client={client}><TranslationSettings userId="owner" /></QueryClientProvider></DakiaProvider>);
  return client;
}
it("lets administrators explicitly save a different execution mode",async()=>{
  const save=vi.spyOn(translationSettingsApi,"save").mockResolvedValue({mode:"device",can_manage:true});
  const client=show(true);
  fireEvent.click(screen.getByRole("radio",{name:/^On devices Translate on/}));
  fireEvent.click(screen.getByRole("button",{name:"Save translation settings"}));
  await screen.findByRole("button", { name: "Saved" });
  expect(save).toHaveBeenCalledWith("device",expect.anything());
  expect(client.getQueryData(translationPolicyKey("owner"))).toEqual({mode:"device",can_manage:true});
  client.clear();
});
it("shows policy without editing controls to a non-admin",()=>{
  const client=show(false);
  expect(screen.queryByRole("radio")).toBeNull();
  expect(screen.queryByRole("button",{name:"Save translation settings"})).toBeNull();
  expect(screen.getByText(/Your administrator controls this setting/)).toBeTruthy();client.clear();
});
