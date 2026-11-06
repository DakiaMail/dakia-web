import type { CssNode, Declaration } from "css-tree";
import parse from "css-tree/parser";
import generate from "css-tree/generator";
import walk from "css-tree/walker";
import { ident } from "css-tree/utils";

export type ImageMap = Record<string, string>;
export function imageSource(value: string): string | null {
  try {
    const url = new URL(value);
    if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.port) return null;
    url.protocol = "https:";
    return url.href;
  } catch { return null; }
}
export function inertImage(value: string): boolean {
  return /^data:image\/(png|jpeg|gif|webp);base64,[A-Za-z0-9+/=]+$/.test(value);
}

// Presentation is permitted inside the opaque frame. Resource permissions are
// separate: no imports/fonts, executable legacy CSS, or unmediated image URLs.
// Parse values as tokens so escapes/comments cannot conceal URL functions.
function declaration(node: Declaration, images?: ImageMap, discover?: Set<string>): string {
  const decoded = ident.decode(node.property);
  const property = decoded.startsWith("--") ? decoded : decoded.toLowerCase();
  if ((!property.startsWith("--") && !/^[a-z-]+$/.test(property)) || /^(?:behavior|-moz-binding|src)$/.test(property)) return "";
  let valid = true;
  const sources = new Set<string>();
  walk(node.value, value => {
    if (value.type === "Raw") valid = false;
    if (value.type === "Function" && /^(?:expression|image|image-set|-webkit-image-set|paint|element|-moz-element|attr)$/i.test(ident.decode(value.name))) valid = false;
    if (value.type === "Url") {
      // Only image properties can introduce resources. Never trust a sender's
      // data URL, even if it claims to be a raster image.
      if (!/^(?:background(?:-image)?|border-image(?:-source)?|list-style(?:-image)?|(?:-webkit-)?mask(?:-image)?|cursor)$/.test(property)) { valid = false; return; }
      const source = imageSource(value.value);
      if (source) sources.add(source);
      const data = source && images?.[source];
      if (data && inertImage(data)) value.value = data;
      else Object.assign(value, { type: "Identifier", name: "none" });
    }
    if (value.type === "Function" && /^(?:url|src)$/i.test(ident.decode(value.name))) valid = false;
  });
  if (!valid) return "";
  for (const source of sources) discover?.add(source);
  return `${ident.encode(property)}:${generate(node.value)}${node.important ? "!important" : ""}`;
}
function declarations(block: CssNode, images?: ImageMap, discover?: Set<string>): string {
  if (block.type !== "Block" && block.type !== "DeclarationList") return "";
  return block.children.toArray().flatMap(node => node.type === "Declaration" ? [declaration(node, images, discover)].filter(Boolean) : []).join(";");
}
function rules(ast: CssNode, images?: ImageMap, discover?: Set<string>, depth = 0): string {
  if (depth > 16 || (ast.type !== "StyleSheet" && ast.type !== "Block")) return "";
  return ast.children.toArray().map(node => {
    if (node.type === "Rule" && node.prelude.type === "SelectorList") {
      const body = declarations(node.block, images, discover);
      return body ? `${generate(node.prelude)}{${body}}` : "";
    }
    if (node.type === "Atrule" && ["media", "supports", "layer", "container", "scope", "starting-style"].includes(ident.decode(node.name).toLowerCase()) && node.block) {
      const body = rules(node.block, images, discover, depth + 1);
      return body ? `@${node.name}${node.prelude ? ` ${generate(node.prelude)}` : ""}{${body}}` : "";
    }
    if (node.type === "Atrule" && /^(?:-webkit-)?keyframes$/i.test(node.name) && node.block) {
      const frames = node.block.children.toArray().map(frame => {
        if (frame.type !== "Rule") return "";
        const selector = generate(frame.prelude);
        if (!/^(?:(?:from|to|\d+(?:\.\d+)?%)(?:\s*,\s*)?)+$/i.test(selector)) return "";
        return `${selector}{${declarations(frame.block, images, discover)}}`;
      }).join("");
      return frames ? `@${node.name} ${node.prelude ? generate(node.prelude) : ""}{${frames}}` : "";
    }
    return "";
  }).join("");
}
export function sanitizeCss(source: string, sheet = false, images?: ImageMap, discover?: Set<string>): string {
  try {
    const ast = parse(source, { context: sheet ? "stylesheet" : "declarationList", parseCustomProperty: true });
    // Escaping '<' is essential when serialized CSS is placed in an HTML style
    // element. A CSS string containing </style> must never end that element.
    return (sheet ? rules(ast, images, discover) : declarations(ast, images, discover)).replace(/</g, "\\3c ");
  } catch { return ""; }
}
