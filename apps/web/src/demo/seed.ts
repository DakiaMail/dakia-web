import type { Classification, InboxKind, Account, Attachment, Draft, MailboxSettings, MailRule, MessageContent, ModelJob, Operation, Submission } from "@dakia/api-types";
import type { CleanupJob } from "../cleanupJobs";

export interface StoredAttachment { meta: Attachment; bytes: Uint8Array }
export interface DemoState {
  version: 2;
  senderClassifications: Record<string, InboxKind>;
  automaticClassifications: Record<string, Classification>;
  classificationOverrides: Record<string, { kind?: InboxKind | null; added?: import("@dakia/api-types").DetectedType[]; removed?: import("@dakia/api-types").DetectedType[]; attention?: boolean | null }>;
  user: string;
  cursor: number;
  accounts: Account[];
  contents: Record<string, MessageContent>;
  drafts: Record<string, Draft>;
  submissions: Record<string, Submission>;
  attachments: Record<string, StoredAttachment>;
  operations: Record<string, Operation>;
  settings: Record<string, MailboxSettings>;
  jobs: Record<string, ModelJob>;
  cleanups: CleanupJob[];
  rules: MailRule[];
  unsubscribed: string[];
  snoozed: Record<string, { until: string; folders: string[] }>;
}
export const identity = (account: string, id: string) => `${account}/${id}`;
export const defaultSettings: MailboxSettings = { full_imap_exposure_confirmed: true, sent_folder: "Sent", archive_folder: "Archive", trash_folder: "Trash", received_cache_budget: 268435456 };
const escape = (text: string) => text.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
export const paragraphs = (text: string) => text.split("\n\n").map(p => `<p>${escape(p).replaceAll("\n", "<br>")}</p>`).join("");

export function seed(now = Date.now()): DemoState {
  const accounts: Account[] = [
    { id: "work", email: "alex@northline.example", display_name: "Work", provider: "imap", state: "ready", revision: 1 },
    { id: "personal", email: "alex@morgan.example", display_name: "Personal", provider: "gmail", state: "ready", revision: 1 },
  ];
  const state: DemoState = { version: 2, senderClassifications: {}, automaticClassifications: {}, classificationOverrides: {}, user: `sandbox-${crypto.randomUUID()}`, cursor: 1, accounts, contents: {}, drafts: {}, submissions: {}, attachments: {}, operations: {}, settings: {}, jobs: {}, cleanups: [], rules: [], unsubscribed: [], snoozed: {} };
  const scenes = [
    ["work", "Maya Chen", "maya@northline.example", "A few thoughts on the launch", "Hi Alex,\n\nThe updated screens are looking good. I left a few notes in the attached review. Could we make time on Thursday to go through the inbox and search together?\n\nThanks,\nMaya", "People"],
    ["personal", "Daniel Brooks", "daniel@brooks.example", "Lunch on Thursday?", "Hey Alex,\n\nAre you free for lunch on Thursday? There’s a new place near the park. I can get there around 12:30.\n\nDaniel", "People"],
    ["work", "Jordan Singh", "jordan@northline.example", "Project roadmap", "Hi team,\n\nHere’s the plan for the next few weeks:\n\n1. Finish the search improvements.\n2. Review the mobile inbox.\n3. Share the release with our early users.\n\nAlex, could you send your notes before Friday?\n\nJordan", "People"],
    ["personal", "Lena Ortiz", "lena@ortiz.example", "Photos from the weekend", "Hi Alex,\n\nThat was such a good walk. The autumn colours were worth the early start. Next time let’s try the longer route by the coast.\n\nSee you soon,\nLena", "People"],
    ["work", "Sofia Laurent", "sofia@atelier.example", "Les maquettes sont prêtes", "Bonjour Alex,\n\nLes nouvelles maquettes sont prêtes pour notre réunion de jeudi. Peux-tu regarder la page de recherche avant midi ?\n\nMerci et à bientôt,\nSofia", "People"],
    ["work", "Code Review", "reviews@code.example", "Review requested: faster mailbox search", "Alex,\n\nMaya requested your review of the mailbox search changes. The update adds subject and sender suggestions and keeps the results available while you type.\n\nThe checks have passed. Your review is the last step.", "Updates"],
    ["personal", "North Rail", "tickets@rail.example", "Your weekend train tickets", "Hi Alex,\n\nYour trip is booked.\n\nDeparture: Saturday, 09:15\nSeat: Coach 3, seat 24\n\nPlease arrive ten minutes before departure. Have a good journey.", "Updates"],
    ["work", "Cloud Harbour", "billing@cloud.example", "Your monthly receipt", "Hi Alex,\n\nThanks for your payment.\n\nHosting: €18.00\nStorage: €4.00\nTotal paid: €22.00\n\nYour receipt is attached.\n\nCloud Harbour", "Updates"],
    ["personal", "The Reading Room", "weekly@reading.example", "A slower kind of Sunday", "Good morning,\n\nThis week: a photographer’s quiet observations, a short essay about walking without a destination, and a recipe for the first cold evening of the year.\n\nPut the kettle on. There’s no rush.\n\nThe Reading Room", "Newsletters"],
    ["work", "Field Notes", "letters@fieldnotes.example", "Designing space for attention", "Hello,\n\nA useful interface gives the important things room to breathe. This week we look at three small teams making everyday tools easier to use.\n\nOur favourite detail: a search box that understands what you meant without getting in your way.\n\nUntil next week,\nField Notes", "Newsletters"],
    ["personal", "Mila Morgan", "mila@morgan.example", "Sunday dinner", "Hi Alex,\n\nWe’re cooking on Sunday. Come around six if you can. Bring that bread you mentioned!\n\nMila", "People"],
    ["work", "Noah Reed", "noah@northline.example", "A quieter inbox", "Hi Alex,\n\nI tried grouping the newsletters separately from personal messages. It makes the morning catch-up much easier. Could you try it and let me know what you think?\n\nNoah", "People"],
  ];
  // A useful range of dates and enough rows to exercise scrolling and folders.
  for (let i = 0; i < 42; i++) {
    const [account, name, email, subject, text, category] = scenes[i % scenes.length];
    const id = `letter-${i}`;
    const previous = i >= 12 && i < 18;
    const folders = i >= 36 ? ["Trash"] : i >= 30 ? ["Archive"] : ["INBOX"];
    const received = new Date(now - (i < 5 ? i * 32 * 60000 : i * 6 * 3600000)).toISOString();
    const finalText = previous ? `Hi Alex,\n\nA quick update on this: everything is on track. Let me know if you have any questions.\n\n${name.split(" ")[0]}` : text;
    const content: MessageContent = {
      message: { id, account_id: account, provider_id: id, thread_id: previous ? `thread-${i - 12}` : `thread-${i}`, folder_ids: folders, sender: { name, email }, to: [{ name: "Alex Morgan", email: accounts.find(a => a.id === account)!.email }], subject: previous ? `Re: ${subject}` : subject, preview: finalText.split("\n\n")[1], received_at: received, unread: i < 9 || i === 19, starred: [0, 3, 11].includes(i), category, category_status: "ready", revision: 1 },
      text: finalText, html: paragraphs(finalText), message_id: `<${id}@northline.example>`, references: previous ? [`<letter-${i - 12}@northline.example>`] : [], attachments: [], reply_to: [], cc: [], detected_language: !previous && i % scenes.length === 4 ? "fr" : "en",
    };
    const classification: Classification = { kind: previous || category === "People" ? "personal" : "automated", status: "ready", types: previous ? [] : category === "Newsletters" ? ["newsletter"] : i % scenes.length === 7 ? ["receipt"] : [], details: [], attention: [], attention_override: null, source: "automatic", policy_revision: "personal-first-1" };
    if (i % scenes.length === 7 && !previous) classification.details = [{ email_type: "receipt", label: "Total", value: "EUR 22.00", action: null }];
    content.message.classification = classification;
    state.automaticClassifications[identity(account, id)] = structuredClone(classification);
    if (i === 0 || i === 7) {
      const filename = i === 0 ? "Launch review.txt" : "Hosting receipt.txt";
      const bytes = new TextEncoder().encode(i === 0 ? "Launch review\n\nInbox: give the sender and subject more room.\nSearch: keep suggestions useful and easy to navigate.\nMobile: check keyboard focus and attachment downloads.\n" : "Cloud Harbour\nHosting: EUR 18.00\nStorage: EUR 4.00\nTotal paid: EUR 22.00\n");
      const meta: Attachment = { id: `attachment-${i}`, filename, content_type: "text/plain", size_bytes: bytes.byteLength };
      state.attachments[identity(account, meta.id)] = { meta, bytes };
      content.attachments.push(meta);
    }
    state.contents[identity(account, id)] = content;
  }
  for (const [id, subject, type, value, label] of [
    ["code", "Your verification code: 482619", "verification_code", "482619", "Verification code"],
    ["flight", "Your flight booking has been cancelled", "flight_booking", "NB472", "Flight changed"],
  ] as const) {
    const c: Classification = { kind: "automated", status: "ready", types: [type], details: [{ email_type: type, label: id === "code" ? "Code" : "Reference", value, action: "copy" }], attention: [{ label, expires_at: id === "code" ? new Date(now + 15 * 60000).toISOString() : null }], attention_override: null, source: "automatic", policy_revision: "personal-first-1" };
    const message = { ...state.contents[identity("work", "letter-0")].message, id, thread_id: id, provider_id: id, account_id: "personal", sender: { name: id === "code" ? "Example Account" : "North Air", email: `${id}@service.example` }, subject, preview: label, received_at: new Date(now - 60000).toISOString(), unread: true, starred: false, classification: c, category: "transactions", to: [{ email: "alex@morgan.example", name: "Alex Morgan" }] };
    state.contents[identity("personal", id)] = { message, text: subject, html: paragraphs(subject), attachments: [], message_id: `<${id}@service.example>`, references: [], reply_to: [], cc: [], detected_language: "en" };
    state.automaticClassifications[identity("personal", id)] = structuredClone(c);
  }
  const draft: Draft = { id: "draft-weekend", account_id: "personal", revision: 1, to: [{ name: "Lena Ortiz", email: "lena@ortiz.example" }], cc: [], bcc: [], subject: "Next weekend", text: "Hi Lena,\n\nShall we try the coastal route next weekend?", html: paragraphs("Hi Lena,\n\nShall we try the coastal route next weekend?"), in_reply_to: null, references: [], attachments: [], saved_at: new Date(now - 3600000).toISOString() };
  state.drafts[identity(draft.account_id, draft.id)] = draft;
  for (const a of accounts) state.settings[a.id] = { ...defaultSettings };
  return state;
}

export const frenchTranslation = "Hi Alex,\n\nThe new designs are ready for our meeting on Thursday. Could you look at the search page before noon?\n\nThank you and see you soon,\nSofia";
