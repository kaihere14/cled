import type { ClipboardBackend, ClipboardStatus } from "../lib/ipc";

const backendNames: Record<ClipboardBackend, string> = {
  windows: "Windows",
  macOs: "macOS",
  wayland: "Wayland",
  x11: "X11",
  xWayland: "XWayland",
  unknown: "Unknown system",
};

export function StatusBadge({ status }: { status: ClipboardStatus | null }) {
  if (!status) return null;

  if (status.state === "unavailable") {
    return <Badge tone="warning" label="Clipboard unavailable" detail={status.reason} />;
  }

  const live = status.changeDetection === "events";
  return (
    <Badge
      tone={status.limited ? "warning" : "ok"}
      label={`${backendNames[status.backend]} · ${live ? "live" : "polling"}`}
      detail={
        live
          ? "The system tells Cled the moment the clipboard changes."
          : "This system has no clipboard change notifications, so Cled checks twice a second."
      }
    />
  );
}

function Badge({ tone, label, detail }: { tone: "ok" | "warning"; label: string; detail: string }) {
  return (
    <span className="flex items-center gap-1.5 text-xs text-neutral-500" title={detail}>
      <span
        aria-hidden
        className={`size-1.5 rounded-full ${tone === "ok" ? "bg-emerald-500" : "bg-amber-500"}`}
      />
      {label}
    </span>
  );
}
