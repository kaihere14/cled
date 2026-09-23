import { type FormEvent, type InputHTMLAttributes, useEffect, useState } from "react";
import {
  type ConnectionMode,
  getSettings,
  type Settings,
  setConnectionMode,
  setRelayUrl,
} from "../lib/ipc";
import { Button } from "./Button";
import { RelayAccount } from "./RelayAccount";
import { RelayConnection } from "./RelayConnection";
import { SegmentedControl } from "./SegmentedControl";

const MODES: { value: ConnectionMode; label: string }[] = [
  { value: "lan", label: "LAN" },
  { value: "relay", label: "Relay" },
];

/**
 * Connection mode, relay connection status, account, and relay URL, as rows of the Settings card.
 * In relay mode, clipboard items also reach paired devices through the relay, end-to-end
 * encrypted, and new devices of the account pair through it (see `RelayJoin`).
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
          <RelayAccount />
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
            <p className="text-xs text-neutral-500">
              Your devices signed in to this account sync through the relay, end-to-end encrypted. A
              new device joins with a one-time code, asked for under Devices.
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
  saved,
  save,
  inputProps,
}: {
  id: string;
  label: string;
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

  return (
    <form onSubmit={submit} noValidate className="flex flex-col gap-2">
      <label htmlFor={id} className="text-sm">
        {label}
      </label>
      <div className="flex gap-2">
        <input
          id={id}
          type="text"
          spellCheck={false}
          autoComplete="off"
          autoCapitalize="off"
          {...inputProps}
          value={draft}
          onChange={(event) => {
            setDraft(event.target.value);
            setError(null);
          }}
          aria-invalid={error !== null}
          aria-describedby={error ? `${id}-error` : undefined}
          className="min-w-0 flex-1 rounded-md border border-neutral-200 bg-white px-3 py-1.5 font-mono text-sm outline-none placeholder:text-neutral-400 focus-visible:border-neutral-400 aria-invalid:border-red-400 dark:border-neutral-800 dark:bg-neutral-900 dark:focus-visible:border-neutral-600 dark:aria-invalid:border-red-500"
        />
        <Button type="submit" disabled={saving || draft === saved}>
          Save
        </Button>
      </div>
      {error && (
        <p id={`${id}-error`} className="text-xs text-red-600 dark:text-red-400">
          {error}
        </p>
      )}
    </form>
  );
}
