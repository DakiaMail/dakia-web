import { describe, expect, it } from "vitest";
import { emptySelection, reconcileSelection, selectMail } from "./selection";

const order = ["work/a", "personal/a", "work/c", "work/d", "work/e"];
const fresh = () => emptySelection("inbox", "list");
const keys = (state: ReturnType<typeof fresh>) => [...state.checked].sort();

describe("visible mail range selection", () => {
  it("starts safely without an anchor and ignores unknown targets", () => {
    const state = selectMail(fresh(), order, order[2], true, true);
    expect(keys(state)).toEqual([order[2]]);
    expect(state.anchor).toBe(order[2]);
    expect(selectMail(state, order, "missing", true, true)).toBe(state);
  });
  it("selects inclusive forward and reversed ranges without moving the anchor", () => {
    let state = selectMail(fresh(), order, order[2], false, true);
    state = selectMail(state, order, order[4], true, true);
    state = selectMail(state, order, order[0], true, true);
    expect(keys(state)).toEqual([...order].sort());
    expect(state.anchor).toBe(order[2]);
  });
  it("removes a checked target's range and preserves unrelated selection", () => {
    let state = selectMail(fresh(), order, order[4], false, true);
    state = selectMail(state, order, order[0], false, true);
    state = selectMail(state, order, order[2], true, true);
    state = selectMail(state, order, order[1], true, false);
    expect(keys(state)).toEqual([order[2], order[4]].sort());
    expect(state.anchor).toBe(order[0]);
    expect(keys(selectMail(state, order, order[4], false, false))).toEqual([order[2]]);
  });
  it("uses supplied display order, not ids or account ordering", () => {
    const display = [order[4], order[2], order[0], order[3], order[1]];
    const state = selectMail(selectMail(fresh(), display, order[2], false, true), display, order[3], true, true);
    expect(keys(state)).toEqual([order[0], order[2], order[3]].sort());
  });
  it("resets checks and anchor on mailbox or filter changes", () => {
    const state = selectMail(fresh(), order, order[0], false, true);
    expect(reconcileSelection(state, order, "search", "list")).toEqual(emptySelection("search", "list"));
  });
  it("preserves checks but resets the anchor on presentation changes", () => {
    const state = selectMail(fresh(), order, order[0], false, true);
    const next = reconcileSelection(state, order, "inbox", "smart");
    expect(keys(next)).toEqual([order[0]]);
    expect(next.anchor).toBeNull();
  });
  it("prunes removed rows and invalid anchors but preserves pagination and reordering", () => {
    const state = selectMail(fresh(), order, order[0], false, true);
    expect(reconcileSelection(state, [...order].reverse(), "inbox", "list")).toBe(state);
    expect(reconcileSelection(state, [...order, "work/f"], "inbox", "list")).toBe(state);
    const next = reconcileSelection(state, order.slice(1), "inbox", "list");
    expect(keys(next)).toEqual([]);
    expect(next.anchor).toBeNull();
    expect(keys(selectMail(next, order.slice(1), order[3], true, true))).toEqual([order[3]]);
  });
  it("never mutates previous checkmarks", () => {
    const state = selectMail(fresh(), order, order[0], false, true);
    selectMail(state, order, order[4], true, true);
    expect(keys(state)).toEqual([order[0]]);
  });
});
