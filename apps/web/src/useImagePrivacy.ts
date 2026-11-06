import { useEffect, useState } from "react";

// Per signed-in user, for all their accounts on this browser. Keep the legacy
// positive key so explicit true AND false choices survive the new default.
export function useImagePrivacy(userId?: string) {
  const [preference, setPreference] = useState<{ user: string; enabled: boolean; hidden: boolean }>();
  useEffect(() => {
    if (!userId) { setPreference(undefined); return; }
    let enabled = true;
    let hidden = false;
    try {
      enabled = window.localStorage.getItem(`dakia:${userId}:always-load-images`) !== "false";
      hidden = window.localStorage.getItem(`dakia:${userId}:block-hidden-images`) === "true";
    } catch { enabled = false; } // Unknown existing choice: do not send requests.
    setPreference({ user: userId, enabled, hidden });
  }, [userId]);
  function save(key: string, value: boolean, field: "enabled" | "hidden") {
    if (!userId || preference?.user !== userId) return;
    // Apply restrictions immediately even when persistence is unavailable.
    if ((field === "enabled" && !value) || (field === "hidden" && value))
      setPreference({ ...preference, [field]: value });
    window.localStorage.setItem(`dakia:${userId}:${key}`, String(value));
    setPreference({ ...preference, [field]: value });
  }
  return {
    alwaysLoadImages: preference?.user === userId && !!preference?.enabled,
    blockHiddenImages: preference?.user === userId && !!preference?.hidden,
    setAlwaysLoadImages(enabled: boolean) { save("always-load-images", enabled, "enabled"); },
    setBlockHiddenImages(enabled: boolean) { save("block-hidden-images", enabled, "hidden"); },
  };
}
