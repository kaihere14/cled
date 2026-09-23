import { type ReactNode, useState } from "react";
import type { HistoryEntry } from "../lib/useClipboard";
import { skippedLabel } from "./describe";
import { ImagePreview } from "./ImagePreview";
import { LockIcon } from "./LockIcon";
import { Section } from "./Section";

const timeFormat = new Intl.DateTimeFormat(undefined, { timeStyle: "medium" });

export function History({
  entries,
  onCopy,
}: {
  entries: HistoryEntry[];
  onCopy: (text: string) => Promise<void>;
}) {
  const [copiedId, setCopiedId] = useState<string | null>(null);

  async function copy(id: string, text: string) {
    await onCopy(text);
    setCopiedId(id);
    setTimeout(() => setCopiedId((current) => (current === id ? null : current)), 1200);
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
              key={entry.key}
              // Entering items fade in and settle from just above. Frequent, so kept short and
              // small; reduced motion keeps the fade and drops the movement.
              className="transition-[opacity,translate] duration-200 ease-out-strong starting:-translate-y-1 starting:opacity-0 motion-reduce:starting:translate-y-0"
            >
              <Row entry={entry} copied={copiedId === entry.key} onCopy={copy} />
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

function Row({
  entry,
  copied,
  onCopy,
}: {
  entry: HistoryEntry;
  copied: boolean;
  onCopy: (key: string, text: string) => void;
}) {
  const { payload } = entry;
  const time = <Time date={entry.copiedAt} copied={copied} origin={originLabel(entry)} />;

  switch (payload.kind) {
    case "text":
      return (
        <button
          type="button"
          onClick={() => onCopy(entry.key, payload.text)}
          title="Copy again"
          className="flex w-full items-start gap-3 rounded-md px-2.5 py-2 text-left transition-[scale,background-color] duration-150 ease-out-strong hover:bg-neutral-100 active:scale-[0.99] dark:hover:bg-neutral-900"
        >
          <span className="line-clamp-2 min-w-0 flex-1 font-mono text-sm break-words whitespace-pre-wrap">
            {payload.text}
          </span>
          {time}
        </button>
      );
    case "image":
      return (
        <StaticRow time={time}>
          <div className="flex items-end gap-2.5">
            <ImagePreview
              url={payload.previewUrl}
              width={payload.width}
              height={payload.height}
              maxHeight={72}
            />
            <span className="text-xs text-neutral-400 tabular-nums">
              {payload.width}×{payload.height}
            </span>
          </div>
        </StaticRow>
      );
    case "skipped":
      return (
        <StaticRow time={time}>
          <span className="flex items-center gap-2 text-sm text-neutral-500">
            <LockIcon className="size-3.5 shrink-0" />
            {skippedLabel(payload.reason)}
          </span>
        </StaticRow>
      );
  }
}

/** Non-interactive row, aligned with the clickable text rows. */
function StaticRow({ children, time }: { children: ReactNode; time: ReactNode }) {
  return (
    <div className="flex w-full items-start gap-3 px-2.5 py-2">
      <div className="min-w-0 flex-1">{children}</div>
      {time}
    </div>
  );
}

/**
 * Where an entry came from. Every entry is from this device until sync exists, so the origin
 * lives in the tooltip rather than repeating the same label on every row.
 */
function originLabel(entry: HistoryEntry): string | undefined {
  switch (entry.origin?.kind) {
    case "thisDevice":
      return "Copied on this device";
    case "otherDevice":
      return `Copied on another device (${entry.origin.deviceId.slice(0, 8)})`;
    default:
      return undefined;
  }
}

/** Timestamp that briefly swaps to "Copied" after a re-copy, crossfading in place. */
function Time({ date, copied, origin }: { date: Date; copied: boolean; origin?: string }) {
  return (
    <span className="relative shrink-0 pt-0.5 text-xs text-neutral-400 tabular-nums" title={origin}>
      <span
        className={`transition-opacity duration-150 ease-out ${copied ? "opacity-0" : "opacity-100"}`}
      >
        {timeFormat.format(date)}
      </span>
      <span
        aria-live="polite"
        className={`absolute inset-0 pt-0.5 text-right text-emerald-600 transition-opacity duration-150 ease-out dark:text-emerald-400 ${copied ? "opacity-100" : "opacity-0"}`}
      >
        {copied ? "Copied" : ""}
      </span>
    </span>
  );
}
