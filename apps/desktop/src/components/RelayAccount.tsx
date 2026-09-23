import { useCallback, useEffect, useState } from "react";
import {
  type AuthStatus,
  authStatus,
  cancelSignIn,
  onAuthChanged,
  signIn,
  signOut,
} from "../lib/ipc";
import { useTauriEvent } from "../lib/listen";
import { Button, SecondaryButton } from "./Button";

/**
 * The account devices sign in with to use the relay, as a row of the Settings card. Signing in
 * happens in the system browser; this only starts it and shows the outcome.
 */
export function RelayAccount() {
  const [status, setStatus] = useState<AuthStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Subscribed before the first fetch below (effects run in order), so no change is missed.
  useTauriEvent(useCallback(() => onAuthChanged(setStatus), []));
  useEffect(() => {
    authStatus()
      .then(setStatus)
      .catch(() => {});
  }, []);

  async function run(action: () => Promise<AuthStatus>) {
    setError(null);
    try {
      setStatus(await action());
    } catch (err) {
      setError(String(err));
    }
  }

  if (!status) return null;

  return (
    <div className="flex flex-col gap-1 px-3 py-2.5">
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0" aria-live="polite">
          {status.state === "signedIn" ? (
            <>
              <p className="truncate text-sm">
                {status.account.email ?? status.account.name ?? "Signed in"}
              </p>
              <p className="text-xs text-neutral-500">
                {status.account.email && status.account.name
                  ? `${status.account.name} · Your devices sync through this account.`
                  : "Your devices sync through this account."}
              </p>
            </>
          ) : status.state === "signingIn" ? (
            <>
              <p className="text-sm">Continue in your browser…</p>
              <p className="text-xs text-neutral-500">Finish signing in, then come back here.</p>
            </>
          ) : (
            <>
              <p className="text-sm">Account</p>
              <p className="text-xs text-neutral-500">
                Sign in on each device with the same account to connect them.
              </p>
            </>
          )}
        </div>
        {status.state === "signedIn" ? (
          <SecondaryButton onClick={() => run(signOut)}>Sign out</SecondaryButton>
        ) : status.state === "signingIn" ? (
          <SecondaryButton onClick={() => cancelSignIn()}>Cancel</SecondaryButton>
        ) : (
          <Button onClick={() => run(signIn)}>Sign in</Button>
        )}
      </div>
      {error && (
        <p className="text-xs text-red-600 transition-opacity duration-200 ease-out-strong starting:opacity-0 dark:text-red-400">
          {error}
        </p>
      )}
    </div>
  );
}
