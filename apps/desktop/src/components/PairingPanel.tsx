import { type FormEvent, useCallback, useEffect, useState } from "react";
import {
  cancelPairing,
  onPaired as onPairedEvent,
  onPairingCode,
  type PairableDevice,
  pairableDevices,
  pairWith,
  startPairing,
} from "../lib/ipc";
import { useTauriEvent } from "../lib/listen";
import { Button } from "./Button";
import { SecondaryButton } from "./Devices";

type Mode = "show" | "enter";

const CODE_LIFETIME_S = 120;

/** Text field for an 8-character pairing code. */
export const CODE_INPUT_CLASS =
  "min-w-0 rounded-md border border-neutral-200 bg-white px-3 py-1.5 text-sm font-mono tracking-widest outline-none placeholder:text-neutral-400 focus-visible:border-neutral-400 dark:border-neutral-700 dark:bg-neutral-900 dark:focus-visible:border-neutral-500";

/** Whether `code` has all 8 characters, with or without the dash. */
export function isCompleteCode(code: string): boolean {
  return code.replace("-", "").length >= 8;
}

/**
 * Pairing, inline in the Devices list. Opened occasionally, so it gets a short enter transition;
 * reduced motion keeps only the fade.
 */
export function PairingPanel({ address, onDone }: { address: string | null; onDone: () => void }) {
  const [mode, setMode] = useState<Mode>("show");
  const [paired, setPaired] = useState<string | null>(null);

  useEffect(() => {
    if (!paired) return;
    const timer = setTimeout(onDone, 1800);
    return () => clearTimeout(timer);
  }, [paired, onDone]);

  return (
    <div className="flex flex-col gap-3 px-3 py-3 transition-[opacity,translate] duration-200 ease-out-strong starting:-translate-y-1 starting:opacity-0 motion-reduce:starting:translate-y-0">
      {paired ? (
        <p className="py-2 text-sm text-emerald-700 dark:text-emerald-300">
          Paired with {paired}. Copies now sync between these devices.
        </p>
      ) : (
        <>
          <ModeSwitch mode={mode} onChange={setMode} />
          {mode === "show" ? <ShowCode onPaired={setPaired} /> : <EnterCode onPaired={setPaired} />}
          <p className="text-xs text-neutral-500">
            Both devices must be on the same network. For devices elsewhere, sign in to the same
            account in relay mode on both, and Cled asks for a code by itself. If your system asks
            whether Cled may use the network, allow it.
            {address && (
              <>
                {" "}
                This device's address: <span className="font-mono">{address}</span>
              </>
            )}
          </p>
          <div className="flex justify-end">
            <SecondaryButton onClick={onDone}>Close</SecondaryButton>
          </div>
        </>
      )}
    </div>
  );
}

function ModeSwitch({ mode, onChange }: { mode: Mode; onChange: (mode: Mode) => void }) {
  const option = (value: Mode, label: string) => (
    <button
      type="button"
      role="tab"
      aria-selected={mode === value}
      onClick={() => onChange(value)}
      className={`flex-1 rounded-md px-3 py-1 text-sm transition-[scale,background-color,color] duration-150 ease-out-strong active:scale-[0.97] ${mode === value ? "bg-white text-neutral-900 shadow-sm dark:bg-neutral-700 dark:text-neutral-100" : "text-neutral-500 hover:text-neutral-800 dark:hover:text-neutral-200"}`}
    >
      {label}
    </button>
  );
  return (
    <div role="tablist" className="flex gap-1 rounded-lg bg-neutral-100 p-1 dark:bg-neutral-800">
      {option("show", "Show my code")}
      {option("enter", "Enter a code")}
    </div>
  );
}

export function ShowCode({ onPaired }: { onPaired: (name: string) => void }) {
  const [code, setCode] = useState<string | null>(null);
  const [expiresAt, setExpiresAt] = useState(0);
  const [now, setNow] = useState(Date.now());
  const [error, setError] = useState<string | null>(null);

  const begin = useCallback(() => {
    setError(null);
    startPairing()
      .then((next) => {
        setCode(next);
        setExpiresAt(Date.now() + CODE_LIFETIME_S * 1000);
      })
      .catch((err) => setError(String(err)));
  }, []);

  useEffect(() => {
    begin();
    return () => {
      cancelPairing();
    };
  }, [begin]);

  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);

  useTauriEvent(
    useCallback(
      () =>
        onPairingCode((next) => {
          setCode(next);
          if (next) setExpiresAt(Date.now() + CODE_LIFETIME_S * 1000);
        }),
      [],
    ),
  );
  useTauriEvent(useCallback(() => onPairedEvent(onPaired), [onPaired]));

  const secondsLeft = Math.max(0, Math.ceil((expiresAt - now) / 1000));
  const expired = code === null || secondsLeft === 0;

  if (error) return <p className="text-sm text-red-600 dark:text-red-400">{error}</p>;

  return (
    <div className="flex flex-col items-center gap-1 py-2">
      {expired ? (
        <>
          <p className="text-sm text-neutral-500">The code expired.</p>
          <SecondaryButton onClick={begin}>New code</SecondaryButton>
        </>
      ) : (
        <>
          <p className="font-mono text-2xl font-semibold tracking-[0.2em] tabular-nums select-all">
            {code}
          </p>
          <p className="text-xs text-neutral-500 tabular-nums">
            Enter this on your other device · {Math.floor(secondsLeft / 60)}:
            {String(secondsLeft % 60).padStart(2, "0")}
          </p>
        </>
      )}
    </div>
  );
}

function EnterCode({ onPaired }: { onPaired: (name: string) => void }) {
  const [devices, setDevices] = useState<PairableDevice[]>([]);
  const [address, setAddress] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Devices showing a code appear on the network within a second or two.
  useEffect(() => {
    let active = true;
    const poll = () =>
      pairableDevices()
        .then((found) => active && setDevices(found))
        .catch(() => {});
    poll();
    const timer = setInterval(poll, 1500);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, []);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      onPaired(await pairWith(address, code));
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  const inputClass =
    "min-w-0 rounded-md border border-neutral-200 bg-white px-3 py-1.5 text-sm outline-none placeholder:text-neutral-400 focus-visible:border-neutral-400 dark:border-neutral-700 dark:bg-neutral-900 dark:focus-visible:border-neutral-500";

  return (
    <form onSubmit={submit} className="flex flex-col gap-2">
      <p className="text-xs text-neutral-500">
        On the other device, choose "Show my code", then pick it here.
      </p>
      {devices.length > 0 ? (
        <div className="flex flex-col gap-1">
          {devices.map((device) => (
            <button
              key={device.id}
              type="button"
              onClick={() => setAddress(device.address)}
              aria-pressed={address === device.address}
              className={`flex items-center justify-between rounded-md border px-3 py-1.5 text-left text-sm transition-[scale,background-color,border-color] duration-150 ease-out-strong active:scale-[0.99] ${address === device.address ? "border-neutral-900 dark:border-neutral-100" : "border-neutral-200 hover:bg-neutral-50 dark:border-neutral-700 dark:hover:bg-neutral-800"}`}
            >
              <span className="truncate">{device.name}</span>
              <span className="font-mono text-xs text-neutral-400">{device.address}</span>
            </button>
          ))}
        </div>
      ) : (
        <p className="text-sm text-neutral-400">Looking for devices showing a code…</p>
      )}
      <input
        value={address}
        onChange={(event) => setAddress(event.target.value)}
        placeholder="Or enter its address, e.g. 192.168.1.20:43117"
        spellCheck={false}
        className={`${inputClass} font-mono`}
      />
      <div className="flex gap-2">
        <input
          value={code}
          onChange={(event) => setCode(event.target.value.toUpperCase())}
          placeholder="XXXX-XXXX"
          maxLength={9}
          autoCapitalize="characters"
          spellCheck={false}
          className={`${CODE_INPUT_CLASS} flex-1`}
        />
        <Button type="submit" disabled={busy || !address.trim() || !isCompleteCode(code)}>
          {busy ? "Pairing…" : "Pair"}
        </Button>
      </div>
      {error && <p className="text-xs text-red-600 dark:text-red-400">{error}</p>}
    </form>
  );
}
