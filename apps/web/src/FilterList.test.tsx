import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { FilterList } from "@dakia/ui";
afterEach(cleanup);
test("properties filter by name and arrows select without stealing native chords", () => {
  const choose = vi.fn();
  const view = render(<FilterList label="Filter type" searchLabel="Find a filter" items={[{ value: "from", label: "From" }, { value: "subject", label: "Subject" }]}
    onSelect={choose} searchIcon={<span />} trailingIcon={<span />} closeControl={<button>Close</button>} />);
  const input = view.getByRole("textbox", { name: "Find a filter" });
  fireEvent.change(input, { target: { value: "sub" } });
  expect(view.queryByRole("button", { name: "From" })).toBeNull();
  for (const modifier of [{ ctrlKey: true }, { metaKey: true }, { altKey: true }, { isComposing: true }]) {
    expect(fireEvent.keyDown(input, { key: "ArrowDown", ...modifier })).toBe(true);
  }
  fireEvent.keyDown(input, { key: "Enter" }); expect(choose).toHaveBeenCalledWith("subject");
  fireEvent.keyDown(input, { key: "ArrowDown" });
  expect(document.activeElement).toBe(view.getByRole("button", { name: "Subject" }));
  fireEvent.click(document.activeElement!); expect(choose).toHaveBeenCalledWith("subject");
  fireEvent.change(input, { target: { value: "missing" } });
  expect(view.getByRole("status").textContent).toBe("No matching filters.");
});
