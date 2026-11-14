import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { SearchField } from "@dakia/ui";
afterEach(cleanup);
function field() {
  const search = vi.fn(), choose = vi.fn(), change = vi.fn();
  const view = render(<SearchField value="launch" label="Find mail" icon={<span />} clearIcon={<span />} onSearch={search} onChange={change}
    options={[{ id: "one", label: "A launch", onSelect: choose }]} />);
  const input = view.getByRole("combobox"); fireEvent.focus(input);
  return { ...view, input, search, choose, change };
}
test("Enter searches until the user explicitly navigates to a suggestion", () => {
  const { input, search, choose } = field();
  fireEvent.keyDown(input, { key: "Enter" }); expect(search).toHaveBeenCalledOnce(); expect(choose).not.toHaveBeenCalled();
  fireEvent.focus(input); fireEvent.keyDown(input, { key: "ArrowDown" }); fireEvent.keyDown(input, { key: "Enter" });
  expect(choose).toHaveBeenCalledOnce();
});
test("Escape closes suggestions without clearing text; Tab leaves normally", () => {
  const { input, change } = field();
  fireEvent.keyDown(input, { key: "Escape" }); expect(input.getAttribute("aria-expanded")).toBe("false"); expect(change).not.toHaveBeenCalled();
  for (const shiftKey of [false, true]) {
    fireEvent.focus(input); fireEvent.keyDown(input, { key: "Tab", shiftKey }); expect(input.getAttribute("aria-expanded")).toBe("false");
  }
});
test("IME and modified keys retain native behavior", () => {
  const { input, search, choose } = field();
  for (const props of [{ isComposing: true }, { ctrlKey: true }, { metaKey: true }, { altKey: true }, { shiftKey: true }]) {
    for (const key of ["Enter", "Escape", "ArrowDown"]) expect(fireEvent.keyDown(input, { key, ...props })).toBe(true);
  }
  expect(search).not.toHaveBeenCalled(); expect(choose).not.toHaveBeenCalled();
});
