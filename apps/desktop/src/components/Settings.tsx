import { type ReactNode, useEffect, useState } from "react";
import { getAutostart, quitApp, setAutostart } from "../lib/ipc";
import { Section } from "./Section";
import { Switch } from "./Switch";

export function Settings() {
  const [autostart, setAutostartState] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getAutostart()
      .then(setAutostartState)
      .catch((err) => setError(String(err)));
  }, []);

  async function toggleAutostart(enabled: boolean) {
    const previous = autostart;
    setAutostartState(enabled); // Optimistic; reverted if the system refuses.
    try {
      await setAutostart(enabled);
      setError(null);
    } catch (err) {
      setAutostartState(previous);
      setError(String(err));
    }
  }

  return (
    <Section title="Settings">
      <div className="divide-y divide-neutral-200 rounded-lg border border-neutral-200 bg-white dark:divide-neutral-800 dark:border-neutral-800 dark:bg-neutral-900">
        <Row
          title="Start Cled when you log in"
          description="Starts in the tray, without opening this window."
        >
          <Switch
            label="Start Cled when you log in"
            checked={autostart ?? false}
            disabled={autostart === null}
            onChange={toggleAutostart}
          />
        </Row>
        <Row
          title="Quit Cled"
          description="Closing the window keeps Cled running in the tray. Quit stops it."
        >
          <button
            type="button"
            onClick={() => quitApp()}
            className="rounded-md border border-neutral-200 px-3 py-1 text-sm transition-[scale,background-color] duration-150 ease-out-strong hover:bg-neutral-100 active:scale-[0.97] dark:border-neutral-700 dark:hover:bg-neutral-800"
          >
            Quit
          </button>
        </Row>
      </div>
      {error && <p className="text-xs text-red-600 dark:text-red-400">{error}</p>}
    </Section>
  );
}

function Row({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-4 px-3 py-2.5">
      <div className="min-w-0">
        <p className="text-sm">{title}</p>
        <p className="text-xs text-neutral-500">{description}</p>
      </div>
      {children}
    </div>
  );
}
