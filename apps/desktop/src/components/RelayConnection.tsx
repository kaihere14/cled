import { useCallback, useEffect, useState } from "react";
import {
  onRelayChanged,
  onRelayMessage,
  type RelayMessage,
  type RelayStatus,
  relayStatus,
  sendRelayTest,
} from "../lib/ipc";
import { useTauriEvent } from "../lib/listen";
import { SecondaryButton } from "./Button";

type Tone = "ok" | "pending" | "warning" | "error";

const dotColors: Record<Tone, string> = {
  ok: "bg-emerald-500",
  pending: "bg-neutral-400 animate-pulse motion-reduce:animate-none",
  warning: "bg-amber-500",
  error: "bg-red-500",
};

function describe(status: RelayStatus): { tone: Tone; label: string; detail: string | null } {
  switch (status.state) {
    case "connected":
      return { tone: "ok", label: "Connected", detail: null };
    case "connecting":
      return { tone: "pending", label: "Connecting…", detail: null };
    case "needsSignIn":
      return { tone: "warning", label: "Sign in to connect", detail: null };
    case "failed":
      return {
        tone: "error",
        label: "Not connected",
        detail:
          status.retryInSecs === null ? status.error : `${status.error} Retrying automatically.`,
      };
    case "off":
      return { tone: "pending", label: "Off", detail: null };
  }
}

/**
 * Relay connection status, as a row of the Settings card, with a TEMPORARY test button that
 * sends a message to this user's other devices. Shown only in relay mode.
 */
export function RelayConnection() {
  const [status, setStatus] = useState<RelayStatus | null>(null);
  const [sending, setSending] = useState(false);
  // `seq` keys each line, so a repeated result or message fades in again and reads as new.
  const [result, setResult] = useState<{ ok: boolean; text: string; seq: number } | null>(null);
  const [received, setReceived] = useState<(RelayMessage & { seq: number }) | null>(null);

  // Subscribed before the first fetch below (effects run in order), so no change is missed.
  useTauriEvent(useCallback(() => onRelayChanged(setStatus), []));
  useTauriEvent(
    useCallback(
      () =>
        onRelayMessage((message) =>
          setReceived((previous) => ({ ...message, seq: (previous?.seq ?? 0) + 1 })),
        ),
      [],
    ),
  );
  useEffect(() => {
    relayStatus()
      .then(setStatus)
      .catch(() => {});
  }, []);

  async function sendTest() {
    setSending(true);
    try {
      const count = await sendRelayTest();
      const text = `Reached ${count} other device${count === 1 ? "" : "s"}.`;
      setResult((previous) => ({ ok: true, text, seq: (previous?.seq ?? 0) + 1 }));
    } catch (err) {
      setResult((previous) => ({ ok: false, text: String(err), seq: (previous?.seq ?? 0) + 1 }));
    } finally {
      setSending(false);
    }
  }

  if (!status) return null;
  const { tone, label, detail } = describe(status);
  const connected = status.state === "connected";

  return (
    <div className="flex flex-col gap-1.5 px-3 py-2.5">
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0" aria-live="polite">
          <p className="flex items-center gap-2 text-sm">
            <span
              aria-hidden
              className={`size-1.5 shrink-0 rounded-full transition-colors duration-200 ${dotColors[tone]}`}
            />
            Relay · {label}
          </p>
          {detail && <p className="text-xs break-words text-neutral-500">{detail}</p>}
        </div>
        <SecondaryButton
          onClick={sendTest}
          disabled={!connected || sending}
          title="Sends a test message to your other devices signed in to this account."
        >
          Send test
        </SecondaryButton>
      </div>

      <div aria-live="polite" className="flex flex-col gap-0.5 empty:hidden">
        {result && (
          <p
            key={result.seq}
            className={`text-xs transition-opacity duration-200 ease-out-strong starting:opacity-0 ${result.ok ? "text-emerald-700 dark:text-emerald-400" : "text-red-600 dark:text-red-400"}`}
          >
            {result.text}
          </p>
        )}
        {received && (
          <p
            key={received.seq}
            className="truncate text-xs text-neutral-500 transition-opacity duration-200 ease-out-strong starting:opacity-0"
            title={`From device ${received.fromDeviceId}`}
          >
            Received: {received.message}
          </p>
        )}
      </div>
    </div>
  );
}
