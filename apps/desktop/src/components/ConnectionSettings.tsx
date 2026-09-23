import { type FormEvent, type InputHTMLAttributes, useEffect, useState } from "react";
import {
  type ConnectionMode,
  getSettings,
  type Settings,
  setConnectionMode,
  setRelayUrl,
  setRelayUserId,
} from "../lib/ipc";
import { Button } from "./Button";
import { RelayConnection } from "./RelayConnection";
import { SegmentedControl } from "./SegmentedControl";

const MODES: { value: ConnectionMode; label: string }[] = [
  { value: "lan", label: "LAN" },
  { value: "relay", label: "Relay" },
];

/**
 * Connection mode, relay connection status, relay URL, and the temporary relay user ID, as rows
 * of the Settings card. In relay mode the app connects to the relay, but clipboard items still
 * sync over the local network only, and the UI says so.
 */
export function ConnectionSettings({ onError }: { onError: (error: string | null) => void }) {
  const [mode, setMode] = useState<ConnectionMode | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);

  useEffect(() => {
    getSettings()
      .then((settings) => {
        setMode(settings.connectionMode);
        setSettings(settings);
      })
      .catch((err) => onError(String(err)));
  }, [onError]);

  async function changeMode(next: ConnectionMode) {
    const previous = mode;
    setMode(next); // Optimistic; reverted if saving fails.
    try {
      await setConnectionMode(next);
      onError(null);
    } catch (err) {
      setMode(previous);
      onError(String(err));
    }
  }

  return (
    <>
      <div className="flex items-center justify-between gap-4 px-3 py-2.5">
        <div className="min-w-0">
          <p className="text-sm">Connection</p>
          <p className="text-xs text-neutral-500">
            {mode === "relay"
              ? "Sync through a relay server, across networks."
              : "Sync with paired devices on the same network."}
          </p>
        </div>
        <SegmentedControl
          label="Connection"
          options={MODES}
          value={mode}
          disabled={mode === null}
          onChange={changeMode}
        />
      </div>

      {mode === "relay" && settings && (
        <div className="flex flex-col transition-opacity duration-200 ease-out-strong starting:opacity-0">
          <RelayConnection />
          <div className="flex flex-col gap-3 px-3 pt-1 pb-2.5">
            <SavedField
              id="relay-url"
              label="Relay URL"
              saved={settings.relayUrl}
              save={async (draft) => {
                const next = await setRelayUrl(draft);
                setSettings(next);
                return next.relayUrl;
              }}
              inputProps={{ type: "url", inputMode: "url" }}
            />
            <SavedField
              id="relay-user-id"
              label="User ID"
              badge="Testing"
              hint="Temporary, until Cled has accounts. Devices with the same ID connect as the same user. It isn't a password: anyone who knows it can join."
              placeholder="e.g. arman-test"
              saved={settings.relayUserId}
              save={async (draft) => {
                const next = await setRelayUserId(draft);
                setSettings(next);
                return next.relayUserId;
              }}
            />
            <p className="text-xs text-amber-700 dark:text-amber-300">
              Clipboard items don't go through the relay yet. Until they do, Cled keeps syncing them
              over your local network.
            </p>
          </div>
        </div>
      )}
    </>
  );
}

/**
 * A text setting with its own Save button. A rejected value keeps the saved one in effect and
 * leaves the draft in place, with the error below it, so it can be corrected.
 */
function SavedField({
  id,
  label,
  badge,
  hint,
  placeholder,
  saved,
  save,
  inputProps,
}: {
  id: string;
  label: string;
  badge?: string;
  hint?: string;
  placeholder?: string;
  saved: string;
  /** Saves the draft and resolves to the stored value; rejects with a message for the user. */
  save: (draft: string) => Promise<string>;
  inputProps?: InputHTMLAttributes<HTMLInputElement>;
}) {
  const [draft, setDraft] = useState(saved);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setSaving(true);
    try {
      setDraft(await save(draft));
      setError(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  }

  const describedBy = error ? `${id}-error` : hint ? `${id}-hint` : undefined;

  return (
    <form onSubmit={submit} noValidate className="flex flex-col gap-2">
      <label htmlFor={id} className="flex items-center gap-2 text-sm">
        {label}
        {badge && (
          <span className="rounded bg-amber-100 px-1.5 py-px text-[11px] font-medium text-amber-800 dark:bg-amber-950 dark:text-amber-300">
            {badge}
          </span>
        )}
      </label>
      <div className="flex gap-2">
        <input
          id={id}
          type="text"
          spellCheck={false}
          autoComplete="off"
          autoCapitalize="off"
          placeholder={placeholder}
          {...inputProps}
          value={draft}
          onChange={(event) => {
            setDraft(event.target.value);
            setError(null);
          }}
          aria-invalid={error !== null}
          aria-describedby={describedBy}
          className="min-w-0 flex-1 rounded-md border border-neutral-200 bg-white px-3 py-1.5 font-mono text-sm outline-none placeholder:text-neutral-400 focus-visible:border-neutral-400 aria-invalid:border-red-400 dark:border-neutral-800 dark:bg-neutral-900 dark:focus-visible:border-neutral-600 dark:aria-invalid:border-red-500"
        />
        <Button type="submit" disabled={saving || draft === saved}>
          Save
        </Button>
      </div>
      {error ? (
        <p id={`${id}-error`} className="text-xs text-red-600 dark:text-red-400">
          {error}
        </p>
      ) : (
        hint && (
          <p id={`${id}-hint`} className="text-xs text-neutral-500">
            {hint}
          </p>
        )
      )}
    </form>
  );
}
