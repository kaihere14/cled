/** On/off switch. The thumb slides; with reduced motion it jumps and only the color fades. */
export function Switch({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`relative h-5 w-9 shrink-0 rounded-full transition-[scale,background-color] duration-150 ease-out-strong active:scale-[0.97] disabled:opacity-40 ${checked ? "bg-emerald-500" : "bg-neutral-300 dark:bg-neutral-700"}`}
    >
      <span
        aria-hidden
        className={`absolute top-0.5 left-0.5 size-4 rounded-full bg-white shadow-sm transition-transform duration-150 ease-out-strong motion-reduce:transition-none ${checked ? "translate-x-4" : "translate-x-0"}`}
      />
    </button>
  );
}
