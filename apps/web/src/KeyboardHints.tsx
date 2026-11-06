import { createContext, useContext, useEffect, useState } from "react";

export const KeyboardHints = createContext(false);
export function KeyHint({ keys }: { keys: string }) {
  return useContext(KeyboardHints) ? <kbd className="key-hint" aria-hidden="true">{keys}</kbd> : null;
}

export const mailShortcuts = [
  ["J / K", "Next / previous email"],
  ["C", "Compose"], ["/", "Search"],
  ["R", "Reply"], ["Shift+R", "Reply all"], ["F", "Forward"],
  ["E", "Archive"], ["Delete", "Move to trash"],
  ["U", "Mark unread"], ["Shift+U", "Mark read"],
  ["S", "Star / unstar"], ["X", "Select / deselect email"],
  ["M", "Move"], ["B", "Snooze"], ["!", "Mark as spam"],
  ["Escape", "Back / clear selection"], ["?", "Keyboard shortcuts"],
] as const;

export function useKeyboardPreferences(userId?: string) {
  const [saved, setSaved] = useState<{ user: string; hints: boolean; shortcuts: boolean }>();
  useEffect(() => {
    if (!userId) { setSaved(undefined); return; }
    let hints = false, shortcuts = true;
    try {
      hints = localStorage.getItem(`dakia:${userId}:keyboard-hints`) === "true";
      shortcuts = localStorage.getItem(`dakia:${userId}:keyboard-shortcuts`) !== "false";
    } catch { /* Defaults also work when storage is unavailable. */ }
    setSaved({ user: userId, hints, shortcuts });
  }, [userId]);
  function save(field: "hints" | "shortcuts", value: boolean) {
    if (!userId || saved?.user !== userId) return;
    localStorage.setItem(`dakia:${userId}:keyboard-${field}`, String(value));
    setSaved({ ...saved, [field]: value });
  }
  return {
    hints: saved?.user === userId && !!saved?.hints,
    shortcuts: saved?.user === userId && !!saved?.shortcuts,
    setHints: (value: boolean) => save("hints", value),
    setShortcuts: (value: boolean) => save("shortcuts", value),
  };
}
