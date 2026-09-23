import type { UnlistenFn } from "@tauri-apps/api/event";
import { useEffect } from "react";

/**
 * Subscribes to a Tauri event for the component's lifetime. Handles the async subscription
 * correctly under StrictMode's mount/unmount/mount.
 */
export function useTauriEvent(subscribe: () => Promise<UnlistenFn>) {
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    subscribe().then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [subscribe]);
}
