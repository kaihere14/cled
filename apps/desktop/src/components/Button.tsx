import type { ButtonHTMLAttributes } from "react";

/** Primary button. Scales down slightly on press so it feels responsive. */
export function Button({ className = "", ...props }: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      className={`rounded-md bg-neutral-900 px-3 py-1.5 text-sm font-medium text-white transition-[scale,background-color] duration-150 ease-out-strong select-none hover:bg-neutral-700 active:scale-[0.97] disabled:pointer-events-none disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-300 ${className}`}
      {...props}
    />
  );
}
