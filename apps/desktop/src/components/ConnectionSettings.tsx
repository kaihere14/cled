import { type FormEvent, useEffect, useState } from "react";
import { type ConnectionMode, getSettings, setConnectionMode, setRelayUrl } from "../lib/ipc";
import { Button } from "./Button";
import { SegmentedControl } from "./SegmentedControl";

const MODES: { value: ConnectionMode; label: string }[] = [
  { value: "lan", label: "LAN" },
  { value: "relay", label: "Relay" },
];

/**
 * Connection mode and relay URL, as rows of the Settings card. Only stores the choice: relay
 * sync isn't implemented yet, so LAN sync keeps running in both modes, and the UI says so.
 */
export function ConnectionSettings({ onError }: { onError: (error: string | null) => void }) {
  const [mode, setMode] = useState<ConnectionMode | null>(null);
  const [savedUrl, setSavedUrl] = useState("");
  const [draftUrl, setDraftUrl] = useState("");
  const [urlError, setUrlError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    getSettings()
      .then((settings) => {
        setMode(settings.connectionMode);
        setSavedUrl(settings.relayUrl);
        setDraftUrl(settings.relayUrl);
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

  async function saveUrl(event: FormEvent) {
    event.preventDefault();
    setSaving(true);
    try {
      const settings = await setRelayUrl(draftUrl);
      setSavedUrl(settings.relayUrl);
      setDraftUrl(settings.relayUrl);
      setUrlError(null);
    } catch (err) {
      // The saved URL stays in effect; the draft is kept so it can be corrected.
      setUrlError(String(err));
    } finally {
      setSaving(false);
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

      {mode === "relay" && (
        <form
          onSubmit={saveUrl}
          noValidate
          className="flex flex-col gap-2 px-3 py-2.5 transition-opacity duration-200 ease-out-strong starting:opacity-0"
        >
          <label htmlFor="relay-url" className="text-sm">
            Relay URL
          </label>
          <div className="flex gap-2">
            <input
              id="relay-url"
              type="url"
              inputMode="url"
              spellCheck={false}
              autoComplete="off"
              value={draftUrl}
              onChange={(event) => {
                setDraftUrl(event.target.value);
                setUrlError(null);
              }}
              aria-invalid={urlError !== null}
              aria-describedby={urlError ? "relay-url-error" : undefined}
              className="min-w-0 flex-1 rounded-md border border-neutral-200 bg-white px-3 py-1.5 font-mono text-sm outline-none placeholder:text-neutral-400 focus-visible:border-neutral-400 aria-invalid:border-red-400 dark:border-neutral-800 dark:bg-neutral-900 dark:focus-visible:border-neutral-600 dark:aria-invalid:border-red-500"
            />
            <Button type="submit" disabled={saving || draftUrl === savedUrl}>
              Save
            </Button>
          </div>
          {urlError ? (
            <p id="relay-url-error" className="text-xs text-red-600 dark:text-red-400">
              {urlError}
            </p>
          ) : (
            <p className="text-xs text-amber-700 dark:text-amber-300">
              Relay sync isn't available yet. Until it is, Cled keeps syncing over your local
              network.
            </p>
          )}
        </form>
      )}
    </>
  );
}
