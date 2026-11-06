import { describe, it, expect, vi, afterEach } from "vitest";
import { sanitizeMail, mailDocument } from "@dakia/mail-renderer";
import { sanitizeAuthored } from "@dakia/mail-composer";
import { api, OperationQueue, optimistic, key } from "./api";
import { parseRecipients, splitQuoted } from "./Composer";
import type { Message, Operation } from "@dakia/api-types";
const m: Message = {
  id: "same-id",
  account_id: "one",
  thread_id: "thread",
  provider_id: "opaque",
  folder_ids: ["INBOX", "Important"],
  sender: { name: null, email: "a@example.com" },
  to: [],
  subject: "Test",
  preview: "",
  received_at: "2026-10-28T10:00:00Z",
  unread: true,
  starred: false,
  category: null,
  category_status: "pending",
  revision: 4,
};
afterEach(() => vi.restoreAllMocks());
describe("received HTML boundary", () => {
  it("preserves hiding declarations and responsive subtrees without exposing preview filler", () => {
    const doc = new DOMParser().parseFromString(sanitizeMail('<style>@media(min-width:600px){.mobile{display:none!important}}</style><div class="mobile" style="display:none">Mobile alternative</div><div style="display:none;font-size:1px;max-height:0;opacity:0;overflow:hidden">Preview</div><p hidden>Hidden</p><table><tr><td style="padding:24px;color:#444">Visible article</td></tr></table>'), "text/html");
    expect(doc.querySelector(".mobile")!.getAttribute("style")).toBe("display:none");
    expect(doc.querySelector("style")!.textContent).toContain("@media");
    expect(doc.querySelector("p")!.hasAttribute("hidden")).toBe(true);
    expect(doc.querySelector("td")!.style.padding).toBe("24px");
  });
  it("removes scripts, forms, frames, remote resources and CSS URLs", () => {
    const html = sanitizeMail(
      '<script>alert(1)</script><form><input name=x></form><iframe src="https://evil.example"></iframe><img src="https://tracker.example" onerror="alert(1)"><div style="position:fixed;background-image:url(https://evil.example);color:red">Safe text</div><svg onload=alert(1)></svg>',
    );
    expect(html).not.toMatch(/script|iframe|input|img|svg|url\(/i);
    expect(html).toContain("Safe text");
    expect(html).toContain("color:red");
  });
  it("removes dangerous links and protects external links", () => {
    const html = sanitizeMail(
      '<a href="javascript:alert(1)">Bad</a><a href="https://example.org">Good</a><a href="//tracker.example">Relative</a>',
    );
    expect(html).not.toContain("javascript:");
    expect(html).not.toContain("//tracker");
    expect(html).toContain("noopener noreferrer");
  });
  it("preserves legacy alignment with inline CSS precedence, never active attribute values", () => {
    const doc = new DOMParser().parseFromString(sanitizeMail('<table><tr><td align="center" style="background:#ee005a;border-radius:20px;padding:0">Offer</td><td align="right" style="text-align:left">Override</td><td align="url(https://tracker.example)">Unsafe</td></tr></table>'), "text/html");
    const cells = doc.querySelectorAll("td");
    expect(cells[0].style.textAlign).toBe("center");
    expect(cells[0].style.backgroundColor).toBe("rgb(238, 0, 90)");
    expect(cells[1].style.textAlign).toBe("left");
    expect(cells[2].getAttribute("style")).toBeNull();
    expect(doc.body.innerHTML).not.toContain("align=\"url");
  });
  it("retains typography, positioning and authored compact table dimensions", () => {
    const doc = new DOMParser().parseFromString(sanitizeMail('<h1 style="font-family:Georgia,serif;margin:0;position:fixed">Article</h1><table width="110" align="center" style="--dakia-table-width:9000px"><tr><td>Icons</td></tr></table><table width="600"><tr><td>Wide</td></tr></table>'), "text/html");
    expect(doc.querySelector("h1")!.style.fontFamily).toBe("Georgia, serif");
    expect(doc.querySelector("h1")!.style.margin).toBe("0px");
    expect(doc.querySelector("h1")!.style.position).toBe("fixed");
    const tables = doc.querySelectorAll("table");
    expect(tables[0].getAttribute("width")).toBe("110");
    expect(tables[0].style.marginLeft).toBe("auto");
    expect(tables[1].getAttribute("width")).toBe("600");
  });
  it("provides a network-denying CSP independently of sanitation", () => {
    expect(mailDocument("<p>Body</p>")).toContain("default-src 'none'");
    expect(mailDocument("")).toContain("img-src 'none'");
  });
  it("retains legacy button fills, rounding and gutters without active CSS", () => {
    const doc = new DOMParser().parseFromString(sanitizeMail('<table cellspacing="0" cellpadding="0" width="100%"><tr><td width="10"></td><td bgcolor="#ff5f00" style="border-radius:24px;padding:8px 16px"><a href="https://example.org/entries" style="color:white;display:inline-block;text-decoration:none">Entries</a></td></tr></table><div bgcolor="red" style="background:#257886 url(https://tracker.example/pixel);position:fixed">Override</div><div bgcolor="url(https://tracker.example/pixel)" width="calc(100%)" style="display:none">Bad</div>'), "text/html");
    expect(doc.querySelector("td[width]")!.getAttribute("width")).toBe("10");
    const cell = doc.querySelectorAll("td")[1];
    expect(cell.style.backgroundColor).toBe("rgb(255, 95, 0)");
    expect(cell.style.borderRadius).toBe("24px");
    expect(doc.querySelector("a")!.style.display).toBe("inline-block");
    expect(doc.querySelector("div")!.style.backgroundColor).toBe("rgb(37, 120, 134)");
    expect(doc.body.innerHTML).not.toMatch(/url\(|calc\(/);
  });
  it("keeps sender HTML on a light baseline regardless of the app theme", () => {
    const html = '<div style="background-color:white"><p>Inherited text</p><a href="https://example.org">Link</a><span style="color:#eee;background-color:#222">Sender colors</span></div>';
    expect(mailDocument(html, true)).toBe(mailDocument(html, false));
    expect(mailDocument(html, true)).toContain("color-scheme:light;background:#FFFFFF");
    expect(mailDocument(html, true)).toContain("color:#20232B");
    expect(mailDocument(html, true)).toContain("color:#eee;background-color:#222");
  });
  it("applies dark defaults only to escaped plain-text documents", () => {
    expect(mailDocument("<p>Plain</p>", true, undefined, undefined, true)).toContain("color-scheme:dark;background:#202126");
    expect(mailDocument("<p>Plain</p>", false, undefined, undefined, true)).toContain("color-scheme:light;background:#FFFFFF");
  });
  it("preserves padding and positioning inside the frame while blocking unmediated CSS resources", () => {
    const doc = new DOMParser().parseFromString(sanitizeMail('<div style="padding:20px 12px;padding-left:8px;position:fixed;background-image:url(https://tracker.example/pixel)">Padded</div>'), "text/html");
    const style = doc.body.firstElementChild!.getAttribute("style")!;
    expect(style).toContain("padding");
    expect(style).not.toMatch(/url\(/);
    expect((doc.body.firstElementChild as HTMLElement).style.paddingLeft).toBe("8px");
  });
  it("constrains authored HTML including paste", () => {
    expect(
      sanitizeAuthored(
        '<div style="background:url(https://evil)"><img src="x"><b>Hello</b><a href="javascript:bad()">link</a></div>',
      ),
    ).toBe("<div><b>Hello</b><a>link</a></div>");
  });
});
describe("mail state", () => {
  it("spam hides Inbox intent without inventing a destination or touching account identity", () => {
    const pending = optimistic({ ...m, folder_ids: ["INBOX", "\\Inbox", "Important"] }, { kind: "spam" });
    expect(pending.folder_ids).toEqual(["Important"]);
    expect(pending.account_id).toBe(m.account_id);
    expect(pending.id).toBe(m.id);
    expect(m.folder_ids).toContain("INBOX");
  });
  it("archive removes inbox membership only", () => {
    expect(optimistic({ ...m, folder_ids: [...m.folder_ids, "\\Inbox"] }, { kind: "archive" }).folder_ids).toEqual([
      "Important",
    ]);
    expect(m.folder_ids).toContain("INBOX");
  });
  it("preserves account IDs and safely encodes opaque path IDs", () => {
    expect(optimistic(m, { kind: "star", value: true }).account_id).toBe("one");
    expect(key("a/b ?")).toBe("a%2Fb%20%3F");
  });
  it("serializes rapid actions with the acknowledged revision", async () => {
    const revisions: number[] = [];
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    vi.spyOn(api, "operate").mockImplementation(async (op) => {
      revisions.push(op.expected_revision);
      if (revisions.length === 1) await gate;
      return { ...op, status: "applied", revision: op.expected_revision + 1 };
    });
    const queue = new OperationQueue();
    const updates = vi.fn();
    const invalidate = vi.fn();
    const first = queue.enqueue(
      m,
      { kind: "star", value: true },
      updates,
      invalidate,
    );
    const second = queue.enqueue(
      { ...m, starred: true },
      { kind: "star", value: false },
      updates,
      invalidate,
    );
    await Promise.resolve();
    expect(updates).toHaveBeenCalledTimes(2);
    release();
    await Promise.all([first, second]);
    expect(revisions).toEqual([4, 5]);
    expect(invalidate).toHaveBeenCalledTimes(1);
  });
  it("does not roll back a newer intent when an older operation fails", async () => {
    let count = 0;
    vi.spyOn(api, "operate").mockImplementation(async (op) => {
      if (++count === 1) throw new Error("offline");
      return { ...op, status: "applied", revision: 5 };
    });
    const queue = new OperationQueue();
    const updates = vi.fn();
    const invalidate = vi.fn();
    const first = queue
      .enqueue(m, { kind: "star", value: true }, updates, invalidate)
      .catch(() => {});
    const second = queue.enqueue(
      { ...m, starred: true },
      { kind: "star", value: false },
      updates,
      invalidate,
    );
    await Promise.all([first, second]);
    expect(updates.mock.calls.at(-1)?.[1].starred).toBe(false);
    expect(invalidate).toHaveBeenCalledTimes(1);
  });
  it("isolates same message IDs in different accounts", async () => {
    const calls: string[] = [];
    vi.spyOn(api, "operate").mockImplementation(async (op: Operation) => {
      calls.push(op.account_id);
      return { ...op, status: "applied", revision: op.expected_revision + 1 };
    });
    const queue = new OperationQueue();
    await Promise.all([
      queue.enqueue(
        m,
        { kind: "read", value: true },
        () => {},
        () => {},
      ),
      queue.enqueue(
        { ...m, account_id: "two" },
        { kind: "read", value: true },
        () => {},
        () => {},
      ),
    ]);
    expect(calls.sort()).toEqual(["one", "two"]);
  });
});
describe("composition", () => {
  it("parses international recipients and rejects header injection", () => {
    expect(parseRecipients("one@example.com;two@example.net")).toHaveLength(2);
    expect(() =>
      parseRecipients("one@example.com\r\nBcc: victim@example.org"),
    ).toThrow();
  });
  it("separates persisted inert quotes from editable authored HTML", () => {
    expect(
      splitQuoted(
        '<div>Reply</div><blockquote data-dakia-quote="true"><p>Original</p></blockquote>',
      ),
    ).toEqual({ html: "<div>Reply</div>", quote: "<p>Original</p>" });
  });
});
