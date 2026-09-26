"use client";

import { AnimatePresence, motion, stagger, type Variants } from "motion/react";
import {
  type ComponentType,
  type KeyboardEvent,
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { type Os, release } from "../site";
import {
  AppleIcon,
  ChevronDownIcon,
  ChevronIcon,
  DownloadIcon,
  LinuxIcon,
  WindowsIcon,
} from "./icons";
import { easeOut } from "./motion";

const OS_ICONS: Record<Os, ComponentType<{ className?: string }>> = {
  macos: AppleIcon,
  windows: WindowsIcon,
  linux: LinuxIcon,
};

type NavigatorWithUAData = Navigator & { userAgentData?: { platform?: string } };

function detectOs(): Os | null {
  const nav = navigator as NavigatorWithUAData;
  const platform = (nav.userAgentData?.platform || nav.platform || "").toLowerCase();
  const ua = nav.userAgent.toLowerCase();

  // Phones and tablets have no desktop build; send them to the release page.
  // iPadOS reports itself as a Mac, so touch support is what gives it away.
  if (
    /android|iphone|ipad|ipod/.test(ua) ||
    (platform.startsWith("mac") && nav.maxTouchPoints > 1)
  ) {
    return null;
  }
  if (platform.startsWith("mac") || ua.includes("mac os")) return "macos";
  if (platform.startsWith("win") || ua.includes("windows")) return "windows";
  if (platform.includes("linux") || ua.includes("linux") || ua.includes("x11")) return "linux";
  return null;
}

const menuItems = (menu: HTMLElement | null) =>
  Array.from(menu?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? []);

// The OS never changes during a visit, so there is nothing to subscribe to.
const subscribe = () => () => {};

// Rows cascade in 30ms apart; the panel itself leaves in one quick fade.
// `custom` is true when the menu opens upward, so it drops toward the button.
const panel: Variants = {
  closed: (up: boolean) => ({
    opacity: 0,
    transform: `translateY(${up ? 4 : -4}px) scale(0.97)`,
  }),
  open: {
    opacity: 1,
    transform: "translateY(0px) scale(1)",
    transition: {
      duration: 0.2,
      ease: easeOut,
      delayChildren: stagger(0.03, { startDelay: 0.04 }),
    },
  },
};

const row: Variants = {
  closed: { opacity: 0, transform: "translateY(4px)" },
  open: { opacity: 1, transform: "translateY(0px)", transition: { duration: 0.2, ease: easeOut } },
};

const VARIANTS = {
  hero: {
    segment:
      "h-12 bg-white text-[15px] font-medium text-indigo-700 hover:bg-indigo-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white",
    main: "gap-2 pr-4 pl-6",
    toggle: "pr-4 pl-3",
    shadow: "shadow-[0_8px_24px_-8px_rgb(20_10_90/0.6)]",
    // Centered under the button so it never runs off a narrow screen.
    anchor: "left-1/2 -translate-x-1/2",
    origin: { down: "origin-top", up: "origin-bottom" },
  },
  nav: {
    segment:
      "h-10 bg-indigo-500 text-[15px] font-medium hover:bg-indigo-400 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-300",
    main: "gap-2 pr-3.5 pl-5",
    toggle: "pr-3.5 pl-2.5",
    shadow: "",
    anchor: "right-0",
    origin: { down: "origin-top-right", up: "origin-bottom-right" },
  },
} as const;

// Tall enough for every row; used to decide whether the menu fits below.
const MENU_HEIGHT = 460;

/**
 * A split button. The main half downloads the installer for the visitor's OS
 * (the server render and unknown platforms fall back to the release page); the
 * arrow opens a menu with every build.
 */
export function DownloadButton({ variant }: { variant: keyof typeof VARIANTS }) {
  const styles = VARIANTS[variant];
  const os = useSyncExternalStore(subscribe, detectOs, () => null);
  const detected = release.platforms.find((platform) => platform.os === os);
  const Icon = os ? OS_ICONS[os] : DownloadIcon;

  const [open, setOpen] = useState(false);
  const [up, setUp] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const toggleRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuId = useId();

  // The page clips its overflow, so a menu that would run off the bottom of the
  // viewport opens upward instead, as long as there is more room above.
  const openMenu = () => {
    const rect = rootRef.current?.getBoundingClientRect();
    if (rect) {
      const below = window.innerHeight - rect.bottom;
      setUp(below < MENU_HEIGHT && rect.top > below);
    }
    setOpen(true);
  };

  const close = (returnFocus: boolean) => {
    setOpen(false);
    if (returnFocus) toggleRef.current?.focus();
  };

  useEffect(() => {
    if (!open) return;
    menuItems(menuRef.current)[0]?.focus();

    const onPointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  const onMenuKeyDown = (event: KeyboardEvent) => {
    const list = menuItems(menuRef.current);
    const index = list.indexOf(document.activeElement as HTMLElement);
    const focusAt = (next: number) => list[(next + list.length) % list.length]?.focus();

    switch (event.key) {
      case "ArrowDown":
        focusAt(index + 1);
        break;
      case "ArrowUp":
        focusAt(index - 1);
        break;
      case "Home":
        focusAt(0);
        break;
      case "End":
        focusAt(list.length - 1);
        break;
      case "Escape":
        close(true);
        break;
      case "Tab":
        setOpen(false);
        return;
      default:
        return;
    }
    event.preventDefault();
  };

  return (
    <div ref={rootRef} className="relative">
      <div className={`flex gap-px rounded-full ${styles.shadow}`}>
        <a
          href={detected?.files[0].href ?? release.page}
          aria-label={detected ? `Download Cled for ${detected.name}` : undefined}
          className={`pressable flex items-center rounded-l-full ${styles.segment} ${styles.main}`}
        >
          <Icon className="size-4 shrink-0" />
          Get Cled
        </a>
        <button
          ref={toggleRef}
          type="button"
          aria-label="All downloads"
          aria-haspopup="menu"
          aria-expanded={open}
          aria-controls={open ? menuId : undefined}
          onClick={() => (open ? setOpen(false) : openMenu())}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown" && !open) {
              event.preventDefault();
              openMenu();
            }
          }}
          className={`pressable flex items-center rounded-r-full ${styles.segment} ${styles.toggle}`}
        >
          <ChevronDownIcon
            className={`size-4 transition-transform duration-200 ease-out-strong motion-reduce:transition-none ${open ? "rotate-180" : ""}`}
          />
        </button>
      </div>

      <div
        className={`absolute z-50 ${up ? "bottom-full mb-2" : "top-full mt-2"} ${styles.anchor}`}
      >
        <AnimatePresence>
          {open && (
            <motion.div
              ref={menuRef}
              id={menuId}
              role="menu"
              aria-label="Downloads"
              onKeyDown={onMenuKeyDown}
              custom={up}
              variants={panel}
              initial="closed"
              animate="open"
              exit={{ opacity: 0, transition: { duration: 0.12, ease: easeOut } }}
              className={`w-72 rounded-2xl border border-white/10 bg-[#100f3a]/95 p-1.5 text-left text-white shadow-[0_24px_48px_-12px_rgb(6_6_26/0.7)] backdrop-blur-xl ${styles.origin[up ? "up" : "down"]}`}
            >
              {release.platforms.map((platform) => {
                const PlatformIcon = OS_ICONS[platform.os];
                return (
                  // biome-ignore lint/a11y/useSemanticElements: ARIA menus group items with role="group"; a <fieldset> is for form controls.
                  <div key={platform.os} role="group" aria-label={platform.name}>
                    <motion.div
                      variants={row}
                      aria-hidden
                      className="flex items-center gap-2 px-2.5 pt-2.5 pb-1 text-xs font-medium text-indigo-200/60"
                    >
                      <PlatformIcon className="size-3.5" />
                      {platform.name}
                      {platform.os === os && (
                        <span className="ml-auto rounded-full bg-indigo-400/15 px-2 py-0.5 text-[11px] text-indigo-200">
                          This device
                        </span>
                      )}
                    </motion.div>
                    {platform.files.map((file) => (
                      <motion.a
                        key={file.href}
                        variants={row}
                        role="menuitem"
                        href={file.href}
                        onClick={() => setOpen(false)}
                        className="group flex items-center justify-between gap-3 rounded-lg px-2.5 py-2 text-sm transition-colors duration-150 outline-none hover:bg-white/[0.07] focus-visible:bg-white/[0.07]"
                      >
                        <span>{file.label}</span>
                        <span className="flex items-center gap-2 font-mono text-xs text-indigo-200/50 transition-colors duration-150 group-hover:text-indigo-100 group-focus-visible:text-indigo-100">
                          {file.detail}
                          <DownloadIcon className="size-3.5" />
                        </span>
                      </motion.a>
                    ))}
                  </div>
                );
              })}

              <motion.div variants={row} className="mt-1.5 border-t border-white/10 pt-1.5">
                <a
                  role="menuitem"
                  href={release.page}
                  onClick={() => setOpen(false)}
                  className="group flex items-center justify-between rounded-lg px-2.5 py-2 text-sm text-indigo-100/70 transition-colors duration-150 outline-none hover:bg-white/[0.07] hover:text-white focus-visible:bg-white/[0.07] focus-visible:text-white"
                >
                  Release notes on GitHub
                  <ChevronIcon className="nudge size-3.5" />
                </a>
              </motion.div>
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </div>
  );
}
