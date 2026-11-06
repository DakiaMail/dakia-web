import { expect, it } from "vitest";
import { remoteImageSources, sanitizeMail } from "@dakia/mail-renderer";

it("excludes explicitly hidden and pixel sources without requesting sender resources", () => {
  const html = '<img src="https://example.org/visible.png" width="18" height="18">' +
    '<img src="https://example.org/visible.png">' +
    '<img src="https://example.org/open" style="display: none; width: 1px; height: 1px;">' +
    '<img src="https://example.org/pixel" width="1">' +
    '<img src="https://example.org/hidden" hidden>' +
    '<img src="https://example.org/invisible" style="visibility:hidden">' +
    '<img src="https://example.org/zero" height="0">' +
    '<img src="http://example.org/insecure">';
  expect(remoteImageSources(html, true)).toEqual(["https://example.org/visible.png", "https://example.org/insecure"]);
  expect(remoteImageSources(html)).toHaveLength(7);
  expect(remoteImageSources('<img src="https://example.org/next">')).toEqual(["https://example.org/next"]);
});
it("upgrades legacy HTTP images to TLS consistently and never renders a remote fallback", () => {
  const png = "data:image/png;base64,iVBORw0KGgo=";
  const html = '<img src="http://example.org/social.png" width="30" height="30"><img src="https://example.org/social.png"><img src="http://example.org:8080/no"><img src="http://user:pass@example.org/no"><img src="//example.org/no">';
  expect(remoteImageSources(html)).toEqual(["https://example.org/social.png"]);
  const clean = sanitizeMail(html, { "https://example.org/social.png": png });
  const images = new DOMParser().parseFromString(clean, "text/html").querySelectorAll("img");
  expect(images).toHaveLength(2);
  expect([...images].every(image => image.getAttribute("src") === png)).toBe(true);
  expect(sanitizeMail(html, {})).not.toContain("img");
});
it("preserves inert image sizing and hidden presentation without permitting active CSS", () => {
  const png = "data:image/png;base64,iVBORw0KGgo=";
  const clean = sanitizeMail('<img src="https://example.org/open" hidden width="1" height="1" style="display:none;width:1px;height:1px;position:fixed;background:url(https://evil.example)">', { "https://example.org/open": png });
  const image = new DOMParser().parseFromString(clean, "text/html").querySelector("img")!;
  expect(image.hasAttribute("hidden")).toBe(true);
  expect(image.getAttribute("width")).toBe("1");
  expect(image.style.display).toBe("none");
  expect(image.style.width).toBe("1px");
  expect(clean).not.toMatch(/evil|url\(/);
});
it("only renders successfully resolved data URLs, never broken placeholders or remote fallback", () => {
  const png = "data:image/png;base64,iVBORw0KGgo=";
  const html = '<img src="https://example.org/good" alt="Good"><img src="https://example.org/bad" alt="Bad"><img src="https://example.org/skip">';
  const clean = sanitizeMail(html, { "https://example.org/good": png, "https://example.org/bad": "https://evil.example/fallback" });
  const doc = new DOMParser().parseFromString(clean, "text/html");
  expect(doc.querySelectorAll("img")).toHaveLength(1);
  expect(doc.querySelector("img")?.getAttribute("src")).toBe(png);
  expect(doc.querySelector("img")?.getAttribute("alt")).toBe("Good");
  expect(sanitizeMail(html)).not.toContain("img");
});
