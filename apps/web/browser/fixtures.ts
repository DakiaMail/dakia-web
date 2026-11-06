import type {
  Account,
  Message,
  MessageContent,
  SearchPage,
  Draft,
  Submission,
} from "@dakia/api-types";
export const accounts: Account[] = [
  {
    id: "work",
    email: "alex@example.com",
    display_name: "Work",
    provider: "imap",
    state: "ready",
    revision: 1,
  },
  {
    id: "personal",
    email: "alex@example.net",
    display_name: "Personal",
    provider: "gmail",
    state: "ready",
    revision: 1,
  },
];
const subjects = [
  "A few thoughts on the launch",
  "Lunch on Thursday?",
  "Re: Project roadmap",
  "Photos from the weekend",
  "Updated contract",
  "Review requested: mail search",
  "New comments on Inbox exploration",
  "Your October receipt",
  "Design review tomorrow",
  "Shared folder updated",
  "Your reservation details",
  "The future of search interfaces",
  "Edition 287: Product momentum",
  "What’s new in October",
  "Building a calmer internet",
];
const senders = [
  "Maya Chen",
  "Daniel Brooks",
  "Jordan Singh",
  "Lena Ortiz",
  "Alex Morgan",
  "GitHub",
  "Figma",
  "Railway",
  "Calendar",
  "Dropbox",
  "Booking",
  "Dense Discovery",
  "The Browser",
  "Linear",
  "Offscreen",
];
export const messages: Message[] = subjects.map((subject, i) => ({
  id: `m${i}`,
  account_id: i % 2 ? "personal" : "work",
  thread_id: `t${i}`,
  provider_id: `p${i}`,
  folder_ids: ["INBOX"],
  sender: { name: senders[i], email: `sender${i}@example.org` },
  to: [
    { email: i % 2 ? "alex@example.net" : "alex@example.com", name: "Alex" },
  ],
  subject,
  preview: [
    "The updated screens are looking good. A few small notes before Friday.",
    "Let me know what you think.",
    "Thanks for the update, here are the details.",
  ][i % 3],
  received_at: "2026-10-28T08:42:00Z",
  unread: i < 12,
  starred: false,
  category: i < 5 ? "People" : i < 11 ? "Updates" : "Newsletters",
  category_status: "ready",
  revision: 1,
}));
export const content: MessageContent = {
  reply_to: [],
  detected_language: null,
  message_id: "fixture@example.invalid",
  references: [],
  cc: [],
  message: messages[0],
  text: "Hi Alex,\n\nThe updated screens are looking good. I have a few small notes before we share them on Friday.\n\nCould we give the inbox a little more breathing room and make the unread messages easier to spot?\n\nI added the details to the attached review. Let me know what you think.\n\nMaya",
  html: "<p>Hi Alex,</p><p>The updated screens are looking good. I have a few small notes before we share them on Friday.</p><p>Could we give the inbox a little more breathing room and make the unread messages easier to spot?</p><p>I added the details to the attached review. Let me know what you think.</p><p>Maya</p>",
  attachments: [
    {
      id: "a1",
      filename: "Launch review.pdf",
      content_type: "application/pdf",
      size_bytes: 479232,
    },
  ],
};
export const page: SearchPage = {
  rows: messages,
  continuation: null,
  coverage: accounts.map((a) => ({
    account_id: a.id,
    folder_ids: ["INBOX"],
    state: "complete",
    error: null,
  })),
  classification: { classified: 15, total: 15 },
  pending: false,
  counts_exact: true,
};
export type FixtureState = {
  drafts: Map<string, Draft>;
  submissions: Submission[];
};
