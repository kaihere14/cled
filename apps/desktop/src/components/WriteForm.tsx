import { type FormEvent, useState } from "react";
import { Button } from "./Button";
import { Section } from "./Section";

export function WriteForm({
  onWrite,
  disabled,
}: {
  onWrite: (text: string) => Promise<void>;
  disabled: boolean;
}) {
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!text) return;
    try {
      await onWrite(text);
      setText("");
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  }

  return (
    <Section title="Write to clipboard">
      <form onSubmit={submit} className="flex gap-2">
        <input
          value={text}
          onChange={(event) => setText(event.target.value)}
          placeholder="Type something to copy"
          disabled={disabled}
          className="min-w-0 flex-1 rounded-md border border-neutral-200 bg-white px-3 py-1.5 text-sm outline-none placeholder:text-neutral-400 focus-visible:border-neutral-400 disabled:opacity-40 dark:border-neutral-800 dark:bg-neutral-900 dark:focus-visible:border-neutral-600"
        />
        <Button type="submit" disabled={disabled || !text}>
          Copy
        </Button>
      </form>
      {error && <p className="text-xs text-red-600 dark:text-red-400">{error}</p>}
    </Section>
  );
}
