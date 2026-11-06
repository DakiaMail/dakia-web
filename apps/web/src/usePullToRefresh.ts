import { useEffect, useRef, useState } from "react";
import type { RefObject } from "react";

export function usePullToRefresh(ref: RefObject<HTMLDivElement | null>, enabled: boolean, refresh: () => Promise<unknown>) {
  const [distance, setDistance] = useState(0);
  const [refreshing, setRefreshing] = useState(false);
  const busy = useRef(false);
  useEffect(() => {
    const element = ref.current;
    if (!element || !enabled) return;
    let start: { x: number; y: number; id: number } | null = null;
    let pull = 0;
    const reset = () => { start = null; pull = 0; setDistance(0); };
    const begin = (event: TouchEvent) => {
      if (!matchMedia("(max-width: 900px)").matches || busy.current || element.scrollTop > 0 || event.touches.length !== 1 || (event.target as Element).closest("input,select,a")) return;
      const touch = event.touches[0];
      start = { x: touch.clientX, y: touch.clientY, id: touch.identifier };
    };
    const move = (event: TouchEvent) => {
      if (!start) return;
      const touch = event.touches[0];
      if (event.touches.length !== 1 || touch.identifier !== start.id || element.scrollTop > 0) { reset(); return; }
      const dy = touch.clientY - start.y;
      if (dy < 0 || Math.abs(touch.clientX - start.x) > Math.max(12, dy)) { reset(); return; }
      if (dy > 8 && event.cancelable) event.preventDefault();
      pull = Math.min(96, dy / 2);
      setDistance(pull);
    };
    const finish = () => {
      const shouldRefresh = start && pull >= 64;
      reset();
      if (!shouldRefresh || busy.current) return;
      busy.current = true;
      setRefreshing(true);
      void refresh().finally(() => { busy.current = false; setRefreshing(false); });
    };
    element.addEventListener("touchstart", begin, { passive: true });
    element.addEventListener("touchmove", move, { passive: false });
    element.addEventListener("touchend", finish);
    element.addEventListener("touchcancel", reset);
    return () => {
      element.removeEventListener("touchstart", begin);
      element.removeEventListener("touchmove", move);
      element.removeEventListener("touchend", finish);
      element.removeEventListener("touchcancel", reset);
    };
  }, [enabled, ref, refresh]);
  return { distance, refreshing };
}
