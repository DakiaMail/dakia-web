import { useEffect, useLayoutEffect, useRef } from "react";

type Close = () => void | boolean | Promise<void | boolean>;
type Frame = { id: string; index: number; parent?: string; kind: "screen" | "layer"; value: unknown; restore: () => void; url: string; reader: boolean; close?: Close; retired?: boolean; placeholder?: boolean };
type Marker = { v: number; id: string; index: number; parent?: string; reader?: boolean; guarded?: boolean };
function marker(value: unknown): Marker | null {
  const m = (value as { dakiaNavigation?: Marker } | null)?.dakiaNavigation;
  return m?.v === 1 && typeof m.id === "string" && Number.isSafeInteger(m.index) ? m : null;
}
function listUrl() {
  const url = new URL(location.href);
  url.searchParams.delete("account"); url.searchParams.delete("message");
  return url.href;
}
export class AppHistory {
  private frames = new Map<string, Frame>();
  private current: Frame;
  private root: Frame;
  private traversing = false;
  private pendingLayers: Array<() => void> = [];
  private guarded = matchMedia("(display-mode: standalone)").matches || (navigator as Navigator & { standalone?: boolean }).standalone === true;
  constructor(value: unknown, restore: () => void) {
    const previous = marker(history.state);
    const readerReload = previous?.reader && new URL(location.href).searchParams.has("message");
    this.root = { id: readerReload ? previous.parent || crypto.randomUUID() : previous?.id || crypto.randomUUID(), index: readerReload ? Math.max(0, previous.index - 1) : previous?.index ?? (this.guarded ? 1 : 0), kind: "screen", value, restore, reader: false, url: listUrl() };
    this.current = this.root; this.frames.set(this.root.id, this.root);
    if (readerReload) {
      this.current = { ...this.root, id: previous.id, index: previous.index, parent: this.root.id, url: location.href, reader: true, placeholder: true };
      this.frames.set(this.current.id, this.current);
      this.write(this.current, false);
    } else if (this.guarded && !previous?.guarded) {
      history.replaceState({ ...history.state, dakiaNavigation: { v: 1, id: "boundary", index: 0, guarded: true } }, "", this.root.url);
      this.write(this.root, true);
    } else this.write(this.root, false);
    window.addEventListener("popstate", this.pop);
  }
  private write(frame: Frame, push: boolean) {
    const state = { ...history.state, dakiaNavigation: { v: 1, id: frame.id, index: frame.index, parent: frame.parent, reader: frame.reader, guarded: this.guarded } };
    if (push) history.pushState(state, "", frame.url); else history.replaceState(state, "", frame.url);
  }
  update(value: unknown, restore: () => void) {
    if (this.traversing || this.current.kind !== "screen" || this.current.placeholder) return;
    this.current.value = value; this.current.restore = restore;
  }
  push(value: unknown, restore: () => void, reader: boolean, url?: string, keepLayer = false) {
    if (this.traversing) return false;
    const replace = this.current.kind === "layer" || this.current.placeholder;
    const layer = this.current.kind === "layer" && keepLayer ? this.current : null;
    if (this.current.kind === "layer" && !layer) this.current.retired = true;
    const frame: Frame = { id: crypto.randomUUID(), index: this.current.index + (replace ? 0 : 1), parent: replace ? this.current.parent : this.current.id, kind: "screen", value, restore, reader, url: url || listUrl() };
    this.frames.set(frame.id, frame); this.current = frame;
    this.write(frame, !replace);
    if (layer) {
      layer.index = frame.index + 1; layer.parent = frame.id; layer.value = value; layer.reader = reader; layer.url = frame.url;
      this.current = layer; this.write(layer, true);
    }
    this.trim(); restore(); return true;
  }
  backTo(predicate: (value: unknown) => boolean) {
    if (this.traversing) return;
    let target = this.current.parent ? this.frames.get(this.current.parent) : undefined;
    while (target && (target.kind !== "screen" || target.retired || !predicate(target.value))) target = target.parent ? this.frames.get(target.parent) : undefined;
    if (target) { this.traversing = true; history.go(target.index - this.current.index); }
    else { this.current = this.root; this.write(this.root, false); this.root.restore(); }
  }
  layer(close: Close): () => void {
    if (this.traversing) {
      // Closing one dialog can immediately open another. Wait for the old
      // entry's asynchronous Back traversal before registering the new layer.
      let cancelled = false, dispose: (() => void) | undefined;
      this.pendingLayers.push(() => { if (!cancelled) dispose = this.layer(close); });
      return () => { cancelled = true; dispose?.(); };
    }
    const frame: Frame = { id: crypto.randomUUID(), index: this.current.index + 1, parent: this.current.id, kind: "layer", value: this.current.value, restore: () => {}, reader: this.current.reader, url: location.href, close };
    this.frames.set(frame.id, frame); this.current = frame; this.write(frame, true);
    return () => {
      frame.retired = true;
      if (this.current.id === frame.id && !this.traversing) { this.traversing = true; history.back(); }
    };
  }
  private pop = (event: PopStateEvent) => {
    this.traversing = false;
    queueMicrotask(() => {
      if (!this.traversing) this.pendingLayers.splice(0).forEach(register => register());
    });
    const previous = this.current, m = marker(event.state);
    let target = m ? this.frames.get(m.id) : undefined;
    if (!target) {
      if (!this.guarded && !m) return; // Ordinary browser Back may leave the site.
      if (this.guarded && m && m.index < previous.index) {
        // Return to the existing root entry instead of pushing another copy.
        // This also preserves any Forward history after closing a reader.
        this.traversing = true;
        history.go(previous.index - m.index);
        return;
      }
      target = { ...this.root, index: (m?.index ?? 0) + (this.guarded ? 1 : 0) };
      this.root = target; this.frames.set(target.id, target); this.current = target;
      this.write(target, this.guarded); target.restore();
    } else {
      if (target.retired) {
        const parent = target.parent ? this.frames.get(target.parent) : undefined;
        target = { ...(parent || this.root), id: target.id, index: target.index, retired: false };
        this.frames.set(target.id, target); this.write(target, false);
      }
      this.current = target; this.write(target, false); target.restore();
    }
    if (previous.kind === "layer" && !previous.retired && previous.id !== target.id) {
      void Promise.resolve().then(() => previous.close?.()).then(result => {
        if (result !== false || previous.retired) return;
        // A draft save failed or an action is busy. Restore one Back entry;
        // never discard the still-mounted dialog or replay the operation.
        previous.index = this.current.index + 1; previous.parent = this.current.id;
        this.current = previous; this.write(previous, true);
      }).catch(() => {
        if (!previous.retired) { previous.index = this.current.index + 1; previous.parent = this.current.id; this.current = previous; this.write(previous, true); }
      });
    }
  };
  private trim() {
    if (this.frames.size <= 200) return;
    for (const [id, frame] of this.frames) {
      if (id !== this.root.id && id !== this.current.id && frame.kind !== "layer") this.frames.delete(id);
      if (this.frames.size <= 200) break;
    }
  }
}
let appHistory: AppHistory | null = null;
export function useAppHistory<T extends { selected: unknown }>(value: T, restore: (value: T) => void, enabled: boolean) {
  const callback = useRef(restore); callback.current = restore;
  useLayoutEffect(() => {
    if (!enabled) return;
    appHistory ||= new AppHistory(value, () => callback.current(value));
    appHistory.update(value, () => callback.current(value));
  });
  return {
    push(next: T, url?: string, previous?: T, keepLayer = false) {
      if (previous) appHistory?.update(previous, () => callback.current(previous));
      if (!appHistory) callback.current(next);
      else appHistory.push(next, () => callback.current(next), !!next.selected, url, keepLayer);
    },
    backTo(predicate: (value: T) => boolean) { appHistory?.backTo(value => predicate(value as T)); },
  };
}
export function useBackLayer(close: Close, enabled = true) {
  const callback = useRef(close); callback.current = close;
  useEffect(() => {
    if (!enabled) return;
    let cancelled = false, dispose: (() => void) | undefined;
    // StrictMode's mount probe must not leave duplicate history entries.
    queueMicrotask(() => { if (!cancelled) dispose = appHistory?.layer(() => callback.current()); });
    return () => { cancelled = true; dispose?.(); };
  }, [enabled]);
}
