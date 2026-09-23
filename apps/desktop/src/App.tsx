import { CurrentClipboard } from "./components/CurrentClipboard";
import { History } from "./components/History";
import { LimitedNotice } from "./components/LimitedNotice";
import { StatusBadge } from "./components/StatusBadge";
import { WriteForm } from "./components/WriteForm";
import { useClipboard } from "./lib/useClipboard";

export function App() {
  const { status, current, history, write } = useClipboard();

  return (
    <main className="mx-auto flex min-h-screen max-w-xl flex-col gap-6 px-5 py-6">
      <header className="flex items-center justify-between">
        <h1 className="text-lg font-semibold tracking-tight">Cled</h1>
        <StatusBadge status={status} />
      </header>

      <LimitedNotice status={status} />

      <CurrentClipboard content={current} />
      <WriteForm onWrite={write} disabled={status?.state !== "watching"} />
      <History entries={history} onCopy={write} />
    </main>
  );
}
