import { afterEach, describe, expect, it } from "vitest";
import { canDismissMail, canUseMailShortcut, plainKey } from "./keyboard";

afterEach(() => {
  document.body.innerHTML = "";
  window.getSelection()?.removeAllRanges();
});

function escape(target: EventTarget, init: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(event);
  return event;
}

describe("keyboard boundaries", () => {
  it("accepts plain keys and explicitly opted-in Shift only", () => {
    expect(plainKey(escape(document.body))).toBe(true);
    const shift = escape(document.body, { shiftKey: true });
    expect(plainKey(shift)).toBe(false);
    expect(plainKey(shift, true)).toBe(true);
    for (const init of [{ ctrlKey: true }, { metaKey: true }, { altKey: true }, { ctrlKey: true, altKey: true }, { isComposing: true }, { keyCode: 229 }]) {
      expect(plainKey(escape(document.body, init), true)).toBe(false);
    }
  });
  it("honors prevented events and ignores repeat or non-element targets", () => {
    const event = escape(document.body); event.preventDefault();
    expect(canDismissMail(event)).toBe(false);
    expect(canDismissMail(escape(document.body, { repeat: true }))).toBe(false);
    expect(canDismissMail(escape(window))).toBe(false);
    expect(canDismissMail(escape(document.body))).toBe(true);
  });
  it("does not dismiss from editors including inherited/empty/plaintext contenteditable", () => {
    for (const html of ['<input>', '<textarea></textarea>', '<select></select>', '<div contenteditable><span>x</span></div>', '<div contenteditable="plaintext-only"><span>x</span></div>', '<div role="textbox"><span>x</span></div>', '<div role="combobox"></div>']) {
      document.body.innerHTML = html;
      expect(canDismissMail(escape(document.querySelector("span") || document.body.firstElementChild!))).toBe(false);
    }
  });
  it("does not dismiss behind a modal even if focus escapes it", () => {
    for (const html of ['<div role="dialog"></div>', '<div role="alertdialog"></div>', '<dialog open></dialog>']) {
      document.body.innerHTML = html;
      expect(canDismissMail(escape(document.body))).toBe(false);
    }
  });
  it("leaves selected text alone", () => {
    document.body.textContent = "selected text";
    const range = document.createRange(); range.selectNodeContents(document.body);
    window.getSelection()!.addRange(range);
    expect(canDismissMail(escape(document.body))).toBe(false);
  });
});


describe("mail shortcut boundaries", () => {
  it("accepts mail buttons but leaves native widgets, editors and popovers alone", () => {
    document.body.innerHTML = '<button>Archive</button>';
    expect(canUseMailShortcut(escape(document.querySelector("button")!))).toBe(true);
    for (const html of ['<input type="checkbox">', '<input>', '<textarea></textarea>', '<select></select>', '<div contenteditable><span>x</span></div>', '<div role="textbox"></div>', '<div role="combobox"></div>', '<div class="message-actions-popover"><button>x</button></div>']) {
      document.body.innerHTML = html;
      expect(canUseMailShortcut(escape(document.querySelector("button,span") || document.body.firstElementChild!))).toBe(false);
    }
  });
  it("ignores repeats, composition, OS chords, prevented events, selections and dialogs", () => {
    for (const init of [{ repeat: true }, { ctrlKey: true }, { metaKey: true }, { altKey: true }, { isComposing: true }, { keyCode: 229 }]) {
      expect(canUseMailShortcut(escape(document.body, init))).toBe(false);
    }
    expect(canUseMailShortcut(escape(document.body, { shiftKey: true }))).toBe(true);
    const prevented = escape(document.body); prevented.preventDefault();
    expect(canUseMailShortcut(prevented)).toBe(false);
    document.body.innerHTML = '<dialog open></dialog>';
    expect(canUseMailShortcut(escape(document.body))).toBe(false);
    document.body.textContent = 'Selected';
    const range = document.createRange(); range.selectNodeContents(document.body);
    window.getSelection()!.addRange(range);
    expect(canUseMailShortcut(escape(document.body))).toBe(false);
  });
});
