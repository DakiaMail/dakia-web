// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { composeLink, mailtoDraft, rememberComposeLink } from "./mailto";
afterEach(() => { sessionStorage.clear(); history.replaceState(null, "", "/"); vi.restoreAllMocks(); });
it("prefills recipients and plain text without turning HTML into active content", () => {
  const d = mailtoDraft("mailto:alex%2Bwork@example.com?to=second@example.com&cc=copy@example.com&bcc=private@example.com&subject=Hello%20there&body=%3Cscript%3E%0D%0AThanks+again&attach=https://evil.invalid", "account");
  expect(d.to.map(a => a.email)).toEqual(["alex+work@example.com", "second@example.com"]);
  expect(d.cc[0].email).toBe("copy@example.com"); expect(d.bcc[0].email).toBe("private@example.com");
  expect(d.subject).toBe("Hello there"); expect(d.text).toBe("<script>\r\nThanks+again");
  expect(d.html).toBe("&lt;script&gt;<br>Thanks+again"); expect(d.attachments).toEqual([]);
  expect(d.revision).toBe(0);
});
it.each(["https://evil.invalid", "mailto:a@example.com%0Abcc:evil@example.com", "mailto:a@example.com?subject=hello%0D%0ABcc:evil@example.com", "mailto:%ZZ", "mailto:not-an-email", `mailto:a@example.com?body=${"x".repeat(16000)}`])("rejects malformed or unsafe input: %s", value => {
  expect(() => mailtoDraft(value, "a")).toThrow();
});
it("consumes email links once, removes private text from the address bar and preserves them through sign-in", () => {
  history.replaceState(null, "", `/?mailto=${encodeURIComponent("mailto:a@example.com?body=Private")}`);
  const link = composeLink(); expect(link).toBe("mailto:a@example.com?body=Private"); expect(location.search).toBe("");
  rememberComposeLink(link); expect(composeLink()).toBe(link); expect(composeLink()).toBeNull();
  history.replaceState(null, "", "/?compose=1"); expect(composeLink()).toBe("mailto:");
});
it("expires saved links and tolerates denied tab storage", () => {
  rememberComposeLink("mailto:a@example.com"); vi.spyOn(Date, "now").mockReturnValue(Date.now() + 600001); expect(composeLink()).toBeNull();
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("denied"); });
  expect(() => rememberComposeLink("mailto:")).not.toThrow();
});
