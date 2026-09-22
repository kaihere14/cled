import type { SkippedReason } from "../lib/ipc";

export function skippedLabel(reason: SkippedReason): string {
  switch (reason.type) {
    case "sensitive":
      return "Skipped: marked private by the app that copied it";
    case "tooLarge":
      return `Skipped: image too large (${reason.width}×${reason.height})`;
  }
}
