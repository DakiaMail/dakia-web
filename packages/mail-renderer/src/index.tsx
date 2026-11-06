import DOMPurify from "dompurify";
import { useEffect, useMemo, useRef, useState } from "react";
import { imageSource, inertImage, sanitizeCss, type ImageMap } from "./css";

// No scripts, forms, embedded documents, SVG or sender-controlled fetches.
// Preserve presentation, including selector hooks and legacy email geometry.
const tags = ["html", "head", "body", "style", "p", "div", "span", "br", "b", "strong", "i", "em", "u", "s", "strike", "small", "big", "sup", "sub", "font", "center", "blockquote", "pre", "code", "ul", "ol", "li", "dl", "dt", "dd", "table", "caption", "colgroup", "col", "thead", "tbody", "tfoot", "tr", "td", "th", "hr", "h1", "h2", "h3", "h4", "h5", "h6", "a", "abbr", "address", "section", "article", "header", "footer", "main", "figure", "figcaption", "wbr", "mark", "time", "bdi", "bdo", "del", "ins", "q", "cite", "samp", "kbd", "var", "ruby", "rt", "rp", "data", "details", "summary"];
const attributes = ["href", "title", "style", "class", "id", "colspan", "rowspan", "span", "dir", "lang", "bgcolor", "color", "face", "size", "align", "valign", "cellpadding", "cellspacing", "border", "width", "height", "hidden", "role", "media", "type", "start", "reversed", "nowrap", "background", "datetime", "open"];
function hiddenImage(node: Element): boolean {
  const style = (node as HTMLElement).style;
  if (node.hasAttribute("hidden") || style.display === "none" || style.visibility === "hidden") return true;
  return [node.getAttribute("width"), node.getAttribute("height"), style.width, style.height]
    .some(value => value != null && /^\s*(?:0(?:\.\d+)?|1(?:\.0+)?)\s*(?:px)?\s*$/i.test(value));
}
function explicitlyHidden(node: Element): boolean {
  for (let current: Element | null = node; current; current = current.parentElement) {
    const style = (current as HTMLElement).style;
    if (current.hasAttribute("hidden") || style?.display === "none" || style?.visibility === "hidden") return true;
  }
  return false;
}
export function remoteImageSources(html: string, blockHiddenImages = false): string[] {
  const sources = new Set<string>();
  // Discovery shares exactly the CSS resource policy used by rendering.
  const doc = sanitizedDocument(html, {}, sources, blockHiddenImages);
  for (const node of doc.querySelectorAll("img")) {
    const src = imageSource(node.getAttribute("data-dakia-source") || "");
    if (src && (!blockHiddenImages || (!hiddenImage(node) && !explicitlyHidden(node)))) sources.add(src);
  }
  return [...sources];
}
function sanitizedDocument(html: string, images?: ImageMap, discover?: Set<string>, blockHiddenImages = false): Document {
  // Remove remote src before parsing the sanitized result into a document.
  DOMPurify.addHook("afterSanitizeAttributes", node => {
    const background = imageSource(node.getAttribute("background") || "");
    node.removeAttribute("background");
    if (background) {
      if (!blockHiddenImages || !explicitlyHidden(node)) discover?.add(background);
      const data = images?.[background];
      if (data && inertImage(data)) node.setAttribute("background", data);
    }
    if (node.tagName !== "IMG") return;
    const source = imageSource(node.getAttribute("src") || "");
    node.removeAttribute("src");
    node.removeAttribute("data-dakia-source");
    if (source && discover) node.setAttribute("data-dakia-source", source);
    const data = source && images?.[source];
    if (data && inertImage(data)) node.setAttribute("src", data);
  });
  let clean: string;
  try {
    clean = DOMPurify.sanitize(html, {
      WHOLE_DOCUMENT: true,
      ALLOWED_TAGS: [...tags, "img"],
      ALLOWED_ATTR: [...attributes, "src", "alt"],
      ALLOW_DATA_ATTR: true,
      ALLOW_ARIA_ATTR: true,
    });
  } finally { DOMPurify.removeHook("afterSanitizeAttributes"); }
  const doc = new DOMParser().parseFromString(clean, "text/html");
  for (const node of doc.querySelectorAll<HTMLElement>("*")) {
    if (node.tagName === "STYLE") {
      node.textContent = sanitizeCss(node.textContent || "", true, images, discover);
      // The type attribute cannot turn this into anything but a stylesheet.
      node.removeAttribute("type");
    }
    const legacy: string[] = [];
    const align = (node.getAttribute("align") || "").toLowerCase();
    if (["left", "center", "right", "justify"].includes(align)) legacy.push(`text-align:${align}`);
    if (node.tagName === "TABLE" && align === "center") legacy.push("margin-left:auto", "margin-right:auto");
    if (node.hasAttribute("align") && !["left", "center", "right", "justify"].includes(align)) node.removeAttribute("align");
    const bgcolor = node.getAttribute("bgcolor") || "";
    if (/^(?:#[a-f0-9]{3,8}|[a-z]+)$/i.test(bgcolor)) legacy.push(`background-color:${bgcolor}`);
    const style = [...legacy, sanitizeCss(node.getAttribute("style") || "", false, images, blockHiddenImages && explicitlyHidden(node) ? undefined : discover)].filter(Boolean).join(";");
    node.removeAttribute("style");
    if (style) node.setAttribute("style", style);
    if (node.tagName === "IMG" && !node.hasAttribute("src") && !discover) node.remove();
    // Legacy presentation attributes only accept their literal grammars.
    for (const attr of ["width", "height", "cellpadding", "cellspacing", "border", "colspan", "rowspan", "span", "size"]) {
      const value = node.getAttribute(attr);
      if (value != null && !/^(?:\d{1,5}(?:\.\d+)?%?|auto|[+-]\d)$/.test(value)) node.removeAttribute(attr);
    }
    for (const attr of ["bgcolor", "color"]) {
      const value = node.getAttribute(attr);
      if (value != null && !/^(?:#[a-f0-9]{3,8}|[a-z]+)$/i.test(value)) node.removeAttribute(attr);
    }
    if (node.tagName === "A") {
      const href = node.getAttribute("href") || "";
      const internal = href.startsWith("#");
      if (!internal && !/^(https?:|mailto:)/i.test(href)) node.removeAttribute("href");
      node.setAttribute("target", internal ? "_self" : "_blank");
      node.setAttribute("rel", "noopener noreferrer");
    }
  }
  return doc;
}
export function sanitizeMail(html: string, images?: ImageMap): string {
  const doc = sanitizedDocument(html, images);
  return [...doc.head.querySelectorAll("style")].map(node => node.outerHTML).join("") + doc.body.innerHTML;
}
function attrs(node: Element): string {
  return [...node.attributes].map(attr => ` ${attr.name}="${escapeHtml(attr.value)}"`).join("");
}
export function mailDocument(html: string, dark = false, images?: Record<string,string>, sizing?: string, plainText = false): string {
  if (sizing && !/^[a-f0-9-]{36}$/i.test(sizing)) throw new Error("Invalid reader channel");
  // Sender HTML is an independent document, not app UI. Mixing dark defaults
  // with retained light sender backgrounds makes inherited text/links unreadable.
  // Keep its light baseline and inline sender colors; only plain text follows UI.
  dark = dark && plainText;
  const document = sanitizedDocument(html, images);
  const senderStyles = [...document.head.querySelectorAll("style")].map(node => node.outerHTML).join("");
  return `<!doctype html><html${attrs(document.documentElement)}><head><meta charset="utf-8"><meta name="referrer" content="no-referrer">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src ${sizing ? `'nonce-${sizing}'` : "'none'"}; style-src 'unsafe-inline'; img-src ${images || sizing ? "data:" : "'none'"}; font-src 'none'; base-uri 'none'; form-action 'none'">
  <style data-dakia-defaults>
  :root{color-scheme:${dark ? "dark" : "light"};background:${dark ? "#202126" : "#FFFFFF"}}
  body{font:16px/${plainText ? "1.7" : "normal"} Arial,sans-serif;color:${dark ? "#F1F2F4" : "#20232B"};margin:0;padding:8px 12px;overflow-wrap:anywhere}
  a{color:${plainText ? (dark ? "#A3A6FF" : "#585CC4") : "inherit"}}
  @media(min-width:601px){body{padding:8px 2px}}
  pre{white-space:pre-wrap;overflow-wrap:anywhere}
  </style>${senderStyles}</head><body${attrs(document.body)}>${document.body.innerHTML}${sizing ? `<script nonce="${sizing}">(() => {
    const report = () => parent.postMessage({kind:'dakia-mail-height',channel:'${sizing}',height:Math.ceil(Math.max(document.body.getBoundingClientRect().height, document.body.scrollHeight)),width:Math.max(document.body.scrollWidth,document.documentElement.scrollWidth),viewport:innerWidth,viewportHeight:innerHeight}, '*');
    addEventListener('message', event => {
      if (event.source !== parent || event.data?.kind !== 'dakia-mail-update' || event.data.channel !== '${sizing}' || typeof event.data.source !== 'string') return;
      const next = new DOMParser().parseFromString(event.data.source, 'text/html');
      next.querySelectorAll('script').forEach(node => node.remove());
      document.head.querySelectorAll('style').forEach(node => node.remove());
      next.head.querySelectorAll('style').forEach(node => document.head.append(node));
      for (const target of [document.documentElement, document.body]) {
        const source = target === document.body ? next.body : next.documentElement;
        [...target.attributes].forEach(attr => target.removeAttribute(attr.name));
        [...source.attributes].forEach(attr => target.setAttribute(attr.name, attr.value));
      }
      document.body.innerHTML = next.body.innerHTML;
      report();
    });
    new ResizeObserver(report).observe(document.body);
    document.addEventListener('load', report, true); addEventListener('resize', report); report();
  })();</script>` : ""}</body></html>`;
}
export function MailReader({
  html,
  text,
  dark = false,
  title = "Message body",
  images,
}: {
  html: string;
  text: string;
  dark?: boolean;
  title?: string;
  images?: Record<string,string>;
}) {
  const frame = useRef<HTMLIFrameElement>(null);
  const holder = useRef<HTMLDivElement>(null);
  const [available, setAvailable] = useState(0);
  const [contentWidth, setContentWidth] = useState(0);
  const [fit, setFit] = useState(true);
  const observedWidth = useRef(0);
  useEffect(() => {
    const measure = () => {
      const width = holder.current?.clientWidth || 0;
      if (observedWidth.current !== width) {
        observedWidth.current = width;
        setAvailable(width);
        setContentWidth(0);
      }
    };
    const observer = new ResizeObserver(measure);
    if (holder.current) observer.observe(holder.current);
    measure();
    return () => observer.disconnect();
  }, []);
  const [channel] = useState(() => crypto.randomUUID());
  const [height, setHeight] = useState(80);
  const lastSize = useRef<{ viewport: number; viewportHeight: number; height: number } | null>(null);
  useEffect(() => {
    const resize = (event: MessageEvent) => {
      if (event.source !== frame.current?.contentWindow || event.origin !== "null") return;
      const value = event.data;
      if (value?.kind !== "dakia-mail-height" || value.channel !== channel || typeof value.height !== "number" || !Number.isFinite(value.height)) return;
      // Reports from an earlier viewport may still be queued after fitting or
      // a pane resize. They cannot size the newly laid-out document.
      if (typeof value.viewport !== "number" || !Number.isFinite(value.viewport) || Math.abs(value.viewport - (frame.current?.clientWidth || 0)) > 1) return;
      if (typeof value.viewportHeight !== "number" || !Number.isFinite(value.viewportHeight) || Math.abs(value.viewportHeight - (frame.current?.clientHeight || 0)) > 1) return;
      const previous = lastSize.current;
      const duplicate = previous && previous.viewport === value.viewport && previous.viewportHeight === value.viewportHeight && previous.height === value.height;
      // A viewport-sized body (or child) can grow by the iframe height change
      // plus its padding on every report. Stop that feedback and retain native
      // frame scrolling for the remaining overflow. Independent content changes
      // at the same viewport, such as a decoded image, can still grow the frame.
      const viewportGrowth = previous ? value.viewportHeight - previous.viewportHeight : 0;
      const feedback = previous && previous.viewport === value.viewport && viewportGrowth > 0 && value.height > previous.height && value.height - previous.height >= viewportGrowth - 1;
      lastSize.current = { viewport: value.viewport, viewportHeight: value.viewportHeight, height: value.height };
      if (!duplicate && !feedback) setHeight(Math.max(48, Math.min(100000, value.height)));
      if (typeof value.width === "number" && Number.isFinite(value.width)) setContentWidth(Math.max(0, Math.min(32000, value.width)));
    };
    window.addEventListener("message", resize);
    return () => window.removeEventListener("message", resize);
  }, [channel]);
  const source = useMemo(
    () =>
      mailDocument(
        html || `<p>${escapeHtml(text).replace(/\n/g, "<br>")}</p>`,
        dark,
        images,
        channel,
        !html,
      ),
    [html, text, dark, images, channel],
  );
  // Updating srcDoc navigates a child frame and adds joint browser history.
  // Keep its sandbox document stable; only parent-sanitized content is updated.
  const initialSource = useRef(source);
  const updateFrame = () => frame.current?.contentWindow?.postMessage({ kind: "dakia-mail-update", channel, source }, "*");
  useEffect(updateFrame, [source, channel]);
  useEffect(() => { lastSize.current = null; setContentWidth(0); }, [source]);
  const wide = available > 0 && contentWidth > available + 1;
  const layoutWidth = fit && wide ? contentWidth : available;
  const scale = fit && wide ? available / layoutWidth : 1;
  return (
    <>
      {wide && <button type="button" className="mail-size-control" onClick={() => setFit(value => !value)} style={{ font: "inherit", fontSize: 12, margin: "8px 0", padding: "4px 8px" }}>{fit ? "Original size" : "Fit to width"}</button>}
      <div ref={holder} style={{ width: "100%", height: height * scale, overflow: "hidden" }}>
        <iframe
          ref={frame}
          style={{ height, minHeight: 0, width: layoutWidth || "100%", maxWidth: "none", transform: scale < 1 ? `scale(${scale})` : undefined, transformOrigin: "top left" }}
          className="mail-body"
          title={title}
          sandbox="allow-scripts allow-popups allow-popups-to-escape-sandbox"
          referrerPolicy="no-referrer"
          srcDoc={initialSource.current}
          onLoad={updateFrame}
        />
      </div>
    </>
  );
}
export function escapeHtml(text: string) {
  return text.replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ]!,
  );
}
