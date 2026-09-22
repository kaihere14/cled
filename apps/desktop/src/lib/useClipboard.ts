import { useCallback, useEffect, useState } from "react";
import {
  type ClipboardPayload,
  type ClipboardStatus,
  clipboardStatus,
  onClipboardChanged,
  readClipboard,
  writeClipboard,
} from "./ipc";

export type HistoryEntry = { id: number; payload: ClipboardPayload; copiedAt: Date };

const HISTORY_LIMIT = 50;
let nextId = 0;

/** Current clipboard, in-memory history of changes seen this session, and a way to write. */
export function useClipboard() {
  const [status, setStatus] = useState<ClipboardStatus | null>(null);
  const [current, setCurrent] = useState<ClipboardPayload | null>(null);
  const [history, setHistory] = useState<HistoryEntry[]>([]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    onClipboardChanged((payload) => {
      setCurrent(payload);
      setHistory((entries) => {
        // Re-copying text already in history moves it to the top instead of duplicating it.
        const rest =
          payload.kind === "text"
            ? entries.filter((e) => !(e.payload.kind === "text" && e.payload.text === payload.text))
            : entries;
        const entry = { id: nextId++, payload, copiedAt: new Date() };
        return [entry, ...rest].slice(0, HISTORY_LIMIT);
      });
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });

    clipboardStatus().then(setStatus);
    readClipboard()
      .then(setCurrent)
      .catch(() => setCurrent(null));

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const write = useCallback((text: string) => writeClipboard(text), []);

  return { status, current, history, write };
}
