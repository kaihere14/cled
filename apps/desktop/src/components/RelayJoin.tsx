import { type FormEvent, type ReactNode, useCallback, useEffect, useState } from "react";
import { type JoinStatus, onRelayJoin, pairThroughRelay, relayJoinStatus } from "../lib/ipc";
import { useTauriEvent } from "../lib/listen";
import { Button } from "./Button";
import { CODE_INPUT_CLASS, isCompleteCode, ShowCode } from "./PairingPanel";

/**
 * Pairing through the relay, as a row of the Devices list. Appears by itself when another device
 * signed in to this account isn't paired with this one: one device shows a code, the other asks
 * for it. The code is the same one-time pairing code as on a local network, so the relay never
 * learns it and can't pair in anyone's place. Rare, so it gets the same short enter transition as
 * the pairing panel; reduced motion keeps only the fade.
 */
export function RelayJoin({ onPaired }: { onPaired: () => void }) {
  const [join, setJoin] = useState<JoinStatus | null>(null);
  const [paired, setPaired] = useState<string | null>(null);

  useTauriEvent(useCallback(() => onRelayJoin(setJoin), []));
  useEffect(() => {
    relayJoinStatus()
      .then(setJoin)
      .catch(() => {});
  }, []);

  const done = useCallback(
    (name: string) => {
      setPaired(name);
      onPaired();
    },
    [onPaired],
  );

  useEffect(() => {
    if (!paired) return;
    const timer = setTimeout(() => setPaired(null), 2500);
    return () => clearTimeout(timer);
  }, [paired]);

  if (paired) {
    return (
      <Row>
        <p className="py-1 text-sm text-emerald-700 dark:text-emerald-300">
          Paired with {paired}. Copies now sync between these devices.
        </p>
      </Row>
    );
  }
  if (!join || (!join.showCode && !join.enterCode)) return null;

  return (
    <Row>
      {join.enterCode && <EnterRelayCode onPaired={done} />}
      {join.showCode && (
        <div className="flex flex-col gap-1">
          <p className="text-sm">A new device on your account wants to sync.</p>
          <ShowCode onPaired={done} />
        </div>
      )}
    </Row>
  );
}

function Row({ children }: { children: ReactNode }) {
  return (
    <div className="flex flex-col gap-3 px-3 py-3 transition-[opacity,translate] duration-200 ease-out-strong starting:-translate-y-1 starting:opacity-0 motion-reduce:starting:translate-y-0">
      {children}
    </div>
  );
}

function EnterRelayCode({ onPaired }: { onPaired: (name: string) => void }) {
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      onPaired(await pairThroughRelay(code));
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-2">
      <div>
        <p className="text-sm">A new device signed in to your account.</p>
        <p className="text-xs text-neutral-500">
          Enter the code it shows to start syncing with it.
        </p>
      </div>
      <div className="flex gap-2">
        <input
          value={code}
          onChange={(event) => setCode(event.target.value.toUpperCase())}
          placeholder="XXXX-XXXX"
          aria-label="Pairing code"
          maxLength={9}
          autoCapitalize="characters"
          spellCheck={false}
          className={`${CODE_INPUT_CLASS} flex-1`}
        />
        <Button type="submit" disabled={busy || !isCompleteCode(code)}>
          {busy ? "Pairing…" : "Pair"}
        </Button>
      </div>
      {error && <p className="text-xs text-red-600 dark:text-red-400">{error}</p>}
    </form>
  );
}
