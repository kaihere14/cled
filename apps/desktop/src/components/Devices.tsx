import { type ReactNode, useCallback, useEffect, useState } from "react";
import { onSyncChanged, removePeer, type SyncStatus, syncStatus } from "../lib/ipc";
import { useTauriEvent } from "../lib/listen";
import { PairingPanel } from "./PairingPanel";
import { Section } from "./Section";

export function Devices() {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [pairing, setPairing] = useState(false);

  const refresh = useCallback(() => {
    syncStatus().then(setStatus);
  }, []);
  useEffect(refresh, [refresh]);
  useTauriEvent(useCallback(() => onSyncChanged(refresh), [refresh]));

  if (!status) return null;

  return (
    <Section title="Devices">
      <div className="divide-y divide-neutral-200 rounded-lg border border-neutral-200 bg-white dark:divide-neutral-800 dark:border-neutral-800 dark:bg-neutral-900">
        <div className="flex items-center justify-between gap-4 px-3 py-2.5">
          <div className="min-w-0">
            <p className="truncate text-sm">{status.deviceName}</p>
            <p className="text-xs text-neutral-500">This device</p>
          </div>
          {!status.unavailable && !pairing && (
            <SecondaryButton onClick={() => setPairing(true)}>Pair a device</SecondaryButton>
          )}
        </div>

        {status.unavailable && (
          <p className="px-3 py-2.5 text-sm text-amber-700 dark:text-amber-300">
            Sync isn't available: {status.unavailable}
          </p>
        )}

        {pairing && (
          <PairingPanel
            address={status.address}
            onDone={() => {
              setPairing(false);
              refresh();
            }}
          />
        )}

        {status.peers.map((peer) => (
          <PeerRow key={peer.id} peer={peer} onRemoved={refresh} />
        ))}

        {!status.unavailable && status.peers.length === 0 && !pairing && (
          <p className="px-3 py-2.5 text-sm text-neutral-400">
            No paired devices yet. Pair one on the same network to share your clipboard.
          </p>
        )}
      </div>
    </Section>
  );
}

function PeerRow({
  peer,
  onRemoved,
}: {
  peer: SyncStatus["peers"][number];
  onRemoved: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function remove() {
    try {
      await removePeer(peer.id);
      onRemoved();
    } catch (err) {
      setError(String(err));
      setConfirming(false);
    }
  }

  return (
    <div className="flex items-center justify-between gap-4 px-3 py-2.5">
      <div className="min-w-0">
        <p className="truncate text-sm">{peer.name}</p>
        <p className="flex items-center gap-1.5 text-xs text-neutral-500">
          <span
            aria-hidden
            className={`size-1.5 rounded-full transition-colors duration-150 ease-out ${peer.online ? "bg-emerald-500" : "bg-neutral-300 dark:bg-neutral-600"}`}
          />
          {error ?? (peer.online ? "Connected" : "Not connected")}
        </p>
      </div>
      {confirming ? (
        <div className="flex shrink-0 items-center gap-2">
          <span className="text-xs text-neutral-500">Remove?</span>
          <SecondaryButton onClick={() => setConfirming(false)}>Cancel</SecondaryButton>
          <button
            type="button"
            onClick={remove}
            className="rounded-md bg-red-600 px-3 py-1 text-sm font-medium text-white transition-[scale,background-color] duration-150 ease-out-strong hover:bg-red-700 active:scale-[0.97]"
          >
            Remove
          </button>
        </div>
      ) : (
        <SecondaryButton onClick={() => setConfirming(true)}>Remove</SecondaryButton>
      )}
    </div>
  );
}

export function SecondaryButton({
  children,
  onClick,
  disabled,
  type = "button",
}: {
  children: ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  type?: "button" | "submit";
}) {
  return (
    <button
      type={type}
      onClick={onClick}
      disabled={disabled}
      className="shrink-0 rounded-md border border-neutral-200 px-3 py-1 text-sm transition-[scale,background-color] duration-150 ease-out-strong hover:bg-neutral-100 active:scale-[0.97] disabled:pointer-events-none disabled:opacity-40 dark:border-neutral-700 dark:hover:bg-neutral-800"
    >
      {children}
    </button>
  );
}
