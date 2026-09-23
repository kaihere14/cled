import type { ClipboardStatus } from "../lib/ipc";

/** Shown when Cled can only partially observe the clipboard. Present from launch, so static. */
export function LimitedNotice({ status }: { status: ClipboardStatus | null }) {
  if (status?.state !== "watching" || !status.limited) return null;

  return (
    <p className="rounded-lg border border-amber-200 bg-amber-50 px-3 py-2.5 text-sm text-amber-900 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-200">
      <span className="font-medium">Limited clipboard access.</span> This desktop (for example
      GNOME) doesn't let background apps watch the clipboard directly, so Cled goes through X11
      compatibility and may miss copies made in some apps.
    </p>
  );
}
