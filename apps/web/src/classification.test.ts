import { expect, it } from "vitest";
import type { Message } from "@dakia/api-types";
import { classification, matchesClassification, needsAttention, optimisticClassification, safeDetailLink } from "./classification";
const message: Message = { id:"one",account_id:"a",provider_id:"one",thread_id:"one",folder_ids:["INBOX"],sender:{email:"alex@example.test",name:"Alex"},to:[],subject:"Hello",preview:"",received_at:"2026-11-12T10:00:00.000Z",unread:false,starred:false,category:"transactions",category_status:"ready",revision:1 };
it("keeps unknown and read mail in Personal without treating old categories as new evidence",()=>{
 expect(matchesClassification(message,{inbox_kind:"personal"})).toBe(true);
 expect(matchesClassification(message,{inbox_kind:"automated"})).toBe(false);
});
it("types overlap independently from Personal and attention",()=>{
 let m=optimisticClassification(message,{kind:"detected_type",value:"flight_booking",present:true});
 m=optimisticClassification(m,{kind:"detected_type",value:"receipt",present:true});
 expect(classification(m).types).toEqual(["flight_booking","receipt"]);
 expect(matchesClassification(m,{inbox_kind:"personal",email_type:"receipt"})).toBe(true);
 expect(needsAttention(m)).toBe(false);
 m=optimisticClassification(m,{kind:"attention",value:true});expect(needsAttention(m)).toBe(true);expect(m.unread).toBe(false);
 m=optimisticClassification(m,{kind:"attention",value:false});expect(needsAttention(m)).toBe(false);
});
it("expiry and dismissal beat detector reasons",()=>{
 const m={...message,classification:{...classification(message),attention:[{label:"Code",expires_at:"2026-11-12T10:15:00.000Z"}]}};
 expect(needsAttention(m,Date.parse("2026-11-12T10:14:00Z"))).toBe(true);
 expect(needsAttention(m,Date.parse("2026-11-12T10:15:00Z"))).toBe(false);
 expect(needsAttention(optimisticClassification(m,{kind:"attention",value:false}),Date.parse("2026-11-12T10:01:00Z"))).toBe(false);
});
it("only exposes explicit public HTTP links",()=>{
 for(const url of ["javascript:alert(1)","http://127.0.0.1/","https://user:pass@host.example","http://[::1]/","http://localhost/","file:///tmp/secret"])expect(safeDetailLink(url)).toBe(null);
 expect(safeDetailLink("https://airline.example/manage")?.hostname).toBe("airline.example");
});
