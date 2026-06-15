import { useEffect, useState } from "react";
import { api, events } from "@/lib/tauri";

export function useLogs(active: boolean) {
  const [lines, setLines] = useState<string[]>([]);

  useEffect(() => {
    if (!active) return;
    let unlisten: (() => void) | undefined;
    let alive = true;
    api.logSnapshot().then((snap) => alive && setLines(snap));
    events.onLogLine((line) => setLines((prev) => [...prev.slice(-499), line])).then((fn) => {
      if (alive) unlisten = fn;
      else fn();
    });
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [active]);

  const clear = async () => {
    await api.clearLogs();
    setLines([]);
  };

  return { lines, clear };
}