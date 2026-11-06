import { useEffect, useRef } from "react";
import Squire from "squire-rte";
import DOMPurify from "dompurify";

export function sanitizeAuthored(html: string): string {
  return DOMPurify.sanitize(html, {
    ALLOWED_TAGS: [
      "div",
      "p",
      "br",
      "b",
      "strong",
      "i",
      "em",
      "u",
      "s",
      "a",
      "ul",
      "ol",
      "li",
      "blockquote",
      "pre",
      "code",
    ],
    ALLOWED_ATTR: ["href", "dir"],
    ALLOW_DATA_ATTR: false,
  });
}
export function RichEditor({
  initialHtml,
  onChange,
}: {
  initialHtml: string;
  onChange: (html: string, text: string) => void;
}) {
  const root = useRef<HTMLDivElement>(null);
  const editor = useRef<Squire | null>(null);
  const change = useRef(onChange);
  change.current = onChange;
  useEffect(() => {
    if (!root.current) return;
    const instance = new Squire(root.current, {
      blockTag: "DIV",
      sanitizeToDOMFragment: (html) =>
        DOMPurify.sanitize(sanitizeAuthored(html), {
          RETURN_DOM_FRAGMENT: true,
        }),
    });
    instance.setHTML(sanitizeAuthored(initialHtml));
    editor.current = instance;
    instance.addEventListener("input", () =>
      change.current(
        sanitizeAuthored(instance.getHTML()),
        root.current?.innerText || "",
      ),
    );
    return () => {
      editor.current = null;
      instance.destroy();
    };
  }, []);
  return (
    <>
      <div className="format-bar" role="toolbar" aria-label="Text formatting">
        {(
          [
            "bold",
            "italic",
            "underline",
            "makeUnorderedList",
            "undo",
            "redo",
          ] as const
        ).map((command, i) => (
          <button
            title={["Bold", "Italic", "Underline", "Bulleted list", "Undo", "Redo"][i]}
            type="button"
            key={command}
            aria-label={
              ["Bold", "Italic", "Underline", "Bulleted list", "Undo", "Redo"][
                i
              ]
            }
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => editor.current?.[command]()}
          >
            {["B", "I", "U", "≡", "↶", "↷"][i]}
          </button>
        ))}
      </div>
      <div
        ref={root}
        className="rich-editor"
        role="textbox"
        aria-label="Message"
        aria-multiline="true"
      />
    </>
  );
}
