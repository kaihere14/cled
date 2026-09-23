import logo from "./assets/logo.png";
import { CurrentClipboard } from "./components/CurrentClipboard";
import { Devices } from "./components/Devices";
import { History } from "./components/History";
import { LimitedNotice } from "./components/LimitedNotice";
import { Settings } from "./components/Settings";
import { StatusBadge } from "./components/StatusBadge";
import { WriteForm } from "./components/WriteForm";
import { useClipboard } from "./lib/useClipboard";

export function App() {
  const { status, current, history, write } = useClipboard();

  return (
    <main className="mx-auto flex min-h-screen max-w-xl flex-col gap-6 px-5 py-6">
      <header className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <img
            src={logo}
            alt=""
            width={28}
            height={28}
            className="size-7 select-none"
            draggable={false}
          />
          <h1 className="text-lg font-semibold tracking-tight">Cled</h1>
        </div>
        <StatusBadge status={status} />
      </header>

      <LimitedNotice status={status} />

      <CurrentClipboard content={current} />
      <WriteForm onWrite={write} disabled={status?.state !== "watching"} />
      <Devices />
      <Settings />
      <History entries={history} onCopy={write} />
    </main>
  );
}
