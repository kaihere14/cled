import type { ClipboardPayload } from "../lib/ipc";
import { skippedLabel } from "./describe";
import { ImagePreview } from "./ImagePreview";
import { LockIcon } from "./LockIcon";
import { Section } from "./Section";

export function CurrentClipboard({ content }: { content: ClipboardPayload | null }) {
  return (
    <Section title="Current">
      <div className="rounded-lg border border-neutral-200 bg-white px-3 py-2.5 dark:border-neutral-800 dark:bg-neutral-900">
        <Body content={content} />
      </div>
    </Section>
  );
}

function Body({ content }: { content: ClipboardPayload | null }) {
  if (!content) {
    return <p className="text-sm text-neutral-400">Nothing Cled can show yet. Copy something.</p>;
  }
  switch (content.kind) {
    case "text":
      return (
        <p className="line-clamp-4 font-mono text-sm break-words whitespace-pre-wrap">
          {content.text}
        </p>
      );
    case "image":
      return (
        <div className="flex flex-col items-start gap-1.5">
          <ImagePreview
            url={content.previewUrl}
            width={content.width}
            height={content.height}
            maxHeight={160}
          />
          <span className="text-xs text-neutral-400 tabular-nums">
            {content.width}×{content.height}
          </span>
        </div>
      );
    case "skipped":
      return (
        <p className="flex items-center gap-2 text-sm text-neutral-500">
          <LockIcon className="size-3.5 shrink-0" />
          {skippedLabel(content.reason)}
        </p>
      );
  }
}
