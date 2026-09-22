import { useState } from "react";
import type { HistoryEntry } from "../lib/useClipboard";
import { Section } from "./Section";

const timeFormat = new Intl.DateTimeFormat(undefined, { timeStyle: "medium" });

export function History({
  entries,
  onCopy,
}: {
  entries: HistoryEntry[];
  onCopy: (text: string) => Promise<void>;
}) {
  const [copiedId, setCopiedId] = useState<number | null>(null);

  async function copy(entry: HistoryEntry) {
    await onCopy(entry.text);
    setCopiedId(entry.id);
    setTimeout(() => setCopiedId((id) => (id === entry.id ? null : id)), 1200);
  }

  return (
    <Section title="Recent changes">
      {entries.length === 0 ? (
        <p className="text-sm text-neutral-400">
          Changes appear here while Cled is open. Nothing is saved.
        </p>
      ) : (
        <ul className="flex flex-col gap-1">
          {entries.map((entry) => (
            <li
              key={entry.id}
              // Entering items fade in and settle from just above. Frequent, so kept short and
              // small; reduced motion keeps the fade and drops the movement.
              className="transition-[opacity,translate] duration-200 ease-out-strong starting:-translate-y-1 starting:opacity-0 motion-reduce:starting:translate-y-0"
            >
              <button
                type="button"
                onClick={() => copy(entry)}
                title="Copy again"
                className="group flex w-full items-start gap-3 rounded-md px-2.5 py-2 text-left transition-[scale,background-color] duration-150 ease-out-strong hover:bg-neutral-100 active:scale-[0.99] dark:hover:bg-neutral-900"
              >
                <span className="line-clamp-2 min-w-0 flex-1 font-mono text-sm break-words whitespace-pre-wrap">
                  {entry.text}
                </span>
                <span className="relative shrink-0 pt-0.5 text-xs text-neutral-400 tabular-nums">
                  <span
                    className={`transition-opacity duration-150 ease-out ${copiedId === entry.id ? "opacity-0" : "opacity-100"}`}
                  >
                    {timeFormat.format(entry.copiedAt)}
                  </span>
                  <span
                    aria-live="polite"
                    className={`absolute inset-0 pt-0.5 text-right text-emerald-600 transition-opacity duration-150 ease-out dark:text-emerald-400 ${copiedId === entry.id ? "opacity-100" : "opacity-0"}`}
                  >
                    {copiedId === entry.id ? "Copied" : ""}
                  </span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}
