import type { ClipboardStatus } from "../lib/ipc";

export function StatusBadge({ status }: { status: ClipboardStatus | null }) {
  if (!status) return null;

  const watching = status.state === "watching";
  return (
    <span
      className="flex items-center gap-1.5 text-xs text-neutral-500"
      title={status.state === "unavailable" ? status.reason : undefined}
    >
      <span
        aria-hidden
        className={`size-1.5 rounded-full ${watching ? "bg-emerald-500" : "bg-amber-500"}`}
      />
      {watching ? "Watching clipboard" : "Clipboard unavailable"}
    </span>
  );
}
