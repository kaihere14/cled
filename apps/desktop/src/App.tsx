import { getVersion } from "@tauri-apps/api/app";
import { useEffect, useState } from "react";

export function App() {
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    // Round-trip through the Tauri bridge to prove the UI <-> Rust wiring works.
    getVersion()
      .then(setVersion)
      .catch(() => setVersion(null));
  }, []);

  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-2 bg-neutral-50 text-neutral-900 dark:bg-neutral-950 dark:text-neutral-100">
      <h1 className="text-3xl font-semibold tracking-tight">Cled</h1>
      <p className="text-sm text-neutral-500">Copy once. Paste anywhere.</p>
      <p className="font-mono text-xs text-neutral-400">
        {version ? `v${version}` : "Tauri bridge unavailable"}
      </p>
    </main>
  );
}
