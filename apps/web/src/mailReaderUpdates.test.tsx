import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { MailReader } from "@dakia/mail-renderer";
vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
afterEach(cleanup);
test("initial load keeps the original document; subsequent changes update it once", () => {
  const view = render(<MailReader html="" text="Hello" dark={false} title="Mail" />);
  const frame = view.getByTitle("Mail") as HTMLIFrameElement;
  const post = vi.fn();
  Object.defineProperty(frame, "contentWindow", { configurable: true, value: { postMessage: post } });
  fireEvent.load(frame);
  expect(post).not.toHaveBeenCalled();
  view.rerender(<MailReader html="" text="Hello" dark title="Mail" />);
  expect(post).toHaveBeenCalledTimes(1);
  view.rerender(<MailReader html="" text="Hello" dark title="Mail" />);
  expect(post).toHaveBeenCalledTimes(1);
});
test("a change before iframe load is applied after load", () => {
  const view = render(<MailReader html="<p>First</p>" text="First" dark={false} title="Mail" />);
  const frame = view.getByTitle("Mail") as HTMLIFrameElement;
  const post = vi.fn();
  Object.defineProperty(frame, "contentWindow", { configurable: true, value: { postMessage: post } });
  view.rerender(<MailReader html="<p>Second</p>" text="Second" dark={false} title="Mail" />);
  expect(post).not.toHaveBeenCalled();
  fireEvent.load(frame);
  expect(post).toHaveBeenCalledTimes(1);
  expect(post.mock.calls[0][0].source).toContain("Second");
});
