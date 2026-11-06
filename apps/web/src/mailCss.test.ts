import { describe, expect, it } from "vitest";
import { mailDocument, remoteImageSources, sanitizeMail } from "@dakia/mail-renderer";
import { sanitizeCss } from "../../../packages/mail-renderer/src/css";
const png = "data:image/png;base64,iVBORw0KGgo=";
describe("email presentation and resource boundary", () => {
  it("keeps complete document styling, selector hooks, responsive columns and price semantics", () => {
    const html = '<html lang="en"><head><style>.product{display:inline-block;width:144px} @media(min-width:600px){.product{width:25%!important}}</style></head><body class="receipt" style="margin:0"><div id="item" class="product" style="border-top:4px solid teal;max-width:144px;position:relative">€11<sup>99</sup></div></body></html>';
    const doc = new DOMParser().parseFromString(mailDocument(html), "text/html");
    expect(doc.documentElement.lang).toBe("en");
    expect(doc.body.className).toBe("receipt");
    expect(doc.body.style.margin).toBe("0px");
    expect(doc.querySelector("#item")!.className).toBe("product");
    expect(doc.querySelector("sup")!.textContent).toBe("99");
    expect(doc.querySelector("#item")!.getAttribute("style")).toContain("border-top:4px solid teal");
    expect(doc.head.textContent).toContain("@media (min-width:600px){.product{width:25%!important}}");
    expect(doc.head.textContent).not.toContain("table{display:block");
    expect(doc.head.textContent).not.toContain("white-space:normal!important");
  });
  it("preserves custom properties, gradients, modern layouts and important cascade", () => {
    expect(sanitizeCss('--ink:#123;color:var(--ink);display:grid;grid-template-columns:1fr 2fr;background:linear-gradient(red,blue)!important')).toBe('--ink:#123;color:var(--ink);display:grid;grid-template-columns:1fr 2fr;background:linear-gradient(red,blue)!important');
  });
  it("mediates CSS image resources using the same discovery and data map as img", () => {
    const html = '<style>.hero{background:url(https://images.example.org/hero.png) center/cover #fff}</style><div style="background-image:url(http://images.example.org/card.png)">Card</div>';
    expect(remoteImageSources(html)).toEqual(['https://images.example.org/hero.png', 'https://images.example.org/card.png']);
    const blocked = sanitizeMail(html);
    expect(blocked).not.toContain('https:');
    expect(blocked).not.toContain('http:');
    const loaded = sanitizeMail(html, { 'https://images.example.org/hero.png': png, 'https://images.example.org/card.png': png });
    expect(loaded.match(/data:image\/png/g)).toHaveLength(2);
  });
  it("preserves case-sensitive custom properties and safe animations", () => {
    expect(sanitizeCss('--Ink:red;color:var(--Ink)')).toBe('--Ink:red;color:var(--Ink)');
    expect(sanitizeCss('@keyframes pulse{from{opacity:0}to{opacity:1}}.dot{animation:pulse 1s infinite}', true)).toContain('@keyframes pulse{from{opacity:0}to{opacity:1}}');
  });
  it("mediates legacy background attributes without allowing direct network fallback", () => {
    const html = '<table background="https://images.example.org/tile.png"><tr><td>Legacy</td></tr></table>';
    expect(remoteImageSources(html)).toEqual(['https://images.example.org/tile.png']);
    expect(sanitizeMail(html)).not.toContain('background=');
    expect(sanitizeMail(html, {'https://images.example.org/tile.png':png})).toContain(`background="${png}"`);
  });
  it("honors hidden-image blocking for inline backgrounds and hidden ancestors", () => {
    const html='<div hidden><img src="https://images.example.org/hidden.png"><div style="background:url(https://images.example.org/hidden-bg.png)">Hidden</div></div><table hidden background="https://images.example.org/legacy-hidden.png"><tr><td>Hidden</td></tr></table>';
    expect(remoteImageSources(html, true)).toEqual([]);
    expect(remoteImageSources(html)).toHaveLength(3);
  });
  it("preserves local footnote links without permitting relative external navigation", () => {
    const doc=new DOMParser().parseFromString(sanitizeMail('<a href="#footnote">Footnote</a><p id="footnote">Note</p><a href="//tracker.example">Relative</a>'),'text/html');
    expect(doc.querySelector('a')!.getAttribute('href')).toBe('#footnote');
    expect(doc.querySelector('a')!.getAttribute('target')).toBe('_self');
    expect(doc.querySelectorAll('a')[1].hasAttribute('href')).toBe(false);
  });
  it.each([
    String.raw`background:u\72l(https://evil.example/x)`,
    'background:image-set("https://evil.example/x" 1x)',
    'background:image("https://evil.example/x")',
    'background:-webkit-image-set(url(https://evil.example/x) 1x)',
    String.raw`width:e\78pression(alert(1))`,
    String.raw`-moz-b\69nding:url(https://evil.example/x)`,
    'behavior:url(https://evil.example/x)',
    'content:attr(data-payload url)',
    '--payload:url(https://evil.example/x);background:var(--payload)',
    'background:url(data:image/svg+xml;base64,PHN2Zz4=)',
  ])("rejects active or unmediated CSS: %s", css => {
    const result = sanitizeCss(css);
    expect(result).not.toMatch(/evil|https?:|data:|expression|binding|behavior|image-set|attr\(/i);
  });
  it("drops imports, font loading and unknown at-rules, retaining safe nested media", () => {
    const result = sanitizeCss('@import "https://evil.example/a";@font-face{font-family:x;src:url(https://evil.example/font)}@unknown{a{color:red}}@supports(display:grid){@media(max-width:500px){.card{display:block;color:red}}}', true);
    expect(result).toBe('@supports (display:grid){@media (max-width:500px){.card{display:block;color:red}}}');
  });
  it("cannot break out of a style element through CSS strings or quoted attributes", () => {
    const css = sanitizeCss('.x{content:"</style><script>alert(1)</script>"}', true);
    const doc = new DOMParser().parseFromString(`<style>${css}</style>`, 'text/html');
    expect(doc.querySelectorAll('script')).toHaveLength(0);
    expect(doc.querySelectorAll('style')).toHaveLength(1);
    const clean = mailDocument('<style>body{color:red}</style><div class="x" onclick="alert(1)"><svg onload="alert(1)"></svg></div><meta http-equiv="refresh" content="0;url=https://evil.example"><base href="https://evil.example"><link rel="stylesheet" href="https://evil.example"><script nonce="fake">alert(1)</script>');
    expect(clean).not.toMatch(/onclick|onload|<svg|<link|<base|http-equiv="refresh"|nonce="fake"/);
    expect(clean).toContain("default-src 'none'");
  });
});
