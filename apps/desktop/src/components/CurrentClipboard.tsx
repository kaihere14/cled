import type { ClipboardPayload } from "../lib/ipc";
import { Section } from "./Section";

export function CurrentClipboard({ content }: { content: ClipboardPayload | null }) {
  return (
    <Section title="Current">
      <div className="rounded-lg border border-neutral-200 bg-white px-3 py-2.5 dark:border-neutral-800 dark:bg-neutral-900">
        {content ? (
          <p className="line-clamp-4 font-mono text-sm break-words whitespace-pre-wrap">
            {content.text}
          </p>
        ) : (
          <p className="text-sm text-neutral-400">Nothing Cled can show yet. Copy some text.</p>
        )}
      </div>
    </Section>
  );
}
