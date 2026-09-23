import { useId } from "react";

/**
 * Pick one of a few options. Native radio inputs underneath, so arrow keys and screen readers
 * work as expected. Only colors change, so there is no motion to reduce.
 */
export function SegmentedControl<T extends string>({
  label,
  options,
  value,
  onChange,
  disabled,
}: {
  label: string;
  options: { value: T; label: string }[];
  value: T | null;
  onChange: (value: T) => void;
  disabled?: boolean;
}) {
  const name = useId();
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className="flex shrink-0 rounded-md bg-neutral-100 p-0.5 dark:bg-neutral-800"
    >
      {options.map((option) => {
        const checked = option.value === value;
        return (
          <label
            key={option.value}
            className={`rounded-[5px] px-2.5 py-0.5 text-sm select-none transition-[scale,background-color,color] duration-150 ease-out-strong active:scale-[0.97] has-focus-visible:outline-2 has-focus-visible:outline-neutral-400 has-disabled:pointer-events-none has-disabled:opacity-40 ${checked ? "bg-white text-neutral-900 shadow-sm dark:bg-neutral-600 dark:text-neutral-100" : "text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100"}`}
          >
            <input
              type="radio"
              name={name}
              value={option.value}
              checked={checked}
              disabled={disabled}
              onChange={() => onChange(option.value)}
              className="sr-only"
            />
            {option.label}
          </label>
        );
      })}
    </div>
  );
}
