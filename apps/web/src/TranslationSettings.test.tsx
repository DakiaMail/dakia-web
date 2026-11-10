import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TranslationSettings } from "./TranslationSettings";
import { translationPolicyKey, translationSettingsApi } from "./translationPolicy";
import * as device from "./deviceTranslation";
afterEach(()=>{cleanup();vi.restoreAllMocks();});
function show(canManage: boolean) {
  const client=new QueryClient({defaultOptions:{queries:{retry:false,staleTime:Infinity}}});
  client.setQueryData(translationPolicyKey("owner"),{mode:"host",can_manage:canManage});
  vi.spyOn(translationSettingsApi,"get").mockResolvedValue({mode:"host",can_manage:canManage});
  vi.spyOn(device,"savedPacks").mockResolvedValue([]);
  render(<QueryClientProvider client={client}><TranslationSettings userId="owner" /></QueryClientProvider>);
  return client;
}
it("lets administrators explicitly save a different execution mode",async()=>{
  const save=vi.spyOn(translationSettingsApi,"save").mockResolvedValue({mode:"device",can_manage:true});
  const client=show(true);
  fireEvent.click(screen.getByRole("radio",{name:/^On devices Translate on/}));
  fireEvent.click(screen.getByRole("button",{name:"Save translation settings"}));
  await screen.findByText("Translation settings saved.");
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
