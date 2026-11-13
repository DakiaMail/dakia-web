import { DakiaProvider } from "@dakia/ui";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { Message } from "@dakia/api-types";
import { MailDetails } from "./MailDetails";
import { classification } from "./classification";
import { page as mail } from "../browser/fixtures";
beforeEach(()=>vi.stubGlobal("matchMedia", vi.fn(()=>({matches:false,addEventListener:vi.fn(),removeEventListener:vi.fn()}))));
afterEach(()=>{cleanup();vi.unstubAllGlobals();});
const message=():Message=>({...mail.rows[0],classification:{...classification(mail.rows[0]),status:"ready",types:["verification_code"],attention:[{label:"Verification code",expires_at:new Date(Date.now()+60000).toISOString()}],details:[{email_type:"verification_code",label:"Code",value:"123456",action:"copy"}]}});
it("copies the exact detected code without opening the message",async()=>{
 const writeText=vi.fn().mockResolvedValue(undefined);vi.stubGlobal("navigator",{clipboard:{writeText}});
 render(<MailDetails message={message()}/>,{wrapper:DakiaProvider});fireEvent.click(screen.getByRole("button",{name:"Code: 123456"}));
 expect(writeText).toHaveBeenCalledWith("123456");await screen.findByText("Copied");
});
it("keeps expired codes copyable and never invents facts for a manually added type",()=>{
 const m=message();m.classification!.attention[0].expires_at="2000-01-01T00:00:00.000Z";m.classification!.types.push("flight_booking");render(<MailDetails message={m}/>,{wrapper:DakiaProvider});
 expect(screen.getByRole("button", {name:"Code: 123456"})).toBeTruthy();expect(screen.queryByText("Codes · Flights")).toBeNull();
});
it("shows the destination domain and rejects unsafe detected links",()=>{
 const m=message();m.classification!.types=["shipping"];m.classification!.details=[{email_type:"shipping",label:"Track shipment",value:"https://carrier.example/track/123",action:"open"},{email_type:"shipping",label:"Unsafe",value:"javascript:alert(1)",action:"open"}];
 render(<MailDetails message={m}/>,{wrapper:DakiaProvider});const link=screen.getByRole("link",{name:"Track shipment carrier.example"});expect(link.getAttribute("rel")).toBe("noopener noreferrer");expect(link.getAttribute("referrerpolicy")).toBe("no-referrer");expect(screen.queryByText("Unsafe")).toBeNull();
});

it("does not turn a manually detected type into a fact or visible category label",()=>{
 const m=message();m.classification!.types=["invoice"];m.classification!.details=[];
 const {container}=render(<MailDetails message={m}/>,{wrapper:DakiaProvider});expect(container.querySelector(".mail-details")).toBeNull();
});
