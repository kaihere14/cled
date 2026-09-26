"use client";

import { LayoutGroup, motion, useInView } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { LockIcon } from "./icons";
import { easeOut, StaggerItem } from "./motion";

const CLIPS = [
  "https://github.com/kaihere14/cled",
  "Meet at 7:30, gate B4",
  "cargo test --workspace",
  "#7FD957",
];

const DEVICES = [
  { name: "MacBook", os: "macOS" },
  { name: "fedora", os: "Linux · Wayland" },
] as const;

type Phase = "idle" | "copied" | "sent";

// How long each phase holds before the next one starts.
const PHASE_MS: Record<Phase, number> = { idle: 1200, copied: 1100, sent: 2200 };

// The first sync waits for the hero's entrance to settle.
const FIRST_SYNC_MS = 2000;

// On-screen movement: strong ease-in-out.
const easeInOut = [0.77, 0, 0.175, 1] as const;

const clipAt = (step: number) => CLIPS[step % CLIPS.length];

/**
 * Two devices taking turns: one copies, the clip travels to the other.
 * The travelling text is a shared layout element, so the eye can follow it
 * from one clipboard into the next.
 */
export function SyncDemo() {
  const ref = useRef<HTMLDivElement>(null);
  const inView = useInView(ref, { amount: 0.5 });
  const pageVisible = usePageVisible();
  const [step, setStep] = useState(1);
  const [phase, setPhase] = useState<Phase>("idle");
  const [started, setStarted] = useState(false);

  useEffect(() => {
    // Pause offscreen and in background tabs; resume where it left off.
    if (!inView || !pageVisible) return;
    const timer = setTimeout(
      () => {
        setStarted(true);
        if (phase === "idle") setPhase("copied");
        else if (phase === "copied") setPhase("sent");
        else {
          setStep((s) => s + 1);
          setPhase("idle");
        }
      },
      started ? PHASE_MS[phase] : FIRST_SYNC_MS,
    );
    return () => clearTimeout(timer);
  }, [inView, pageVisible, phase, started]);

  const source = step % 2;

  const card = (index: 0 | 1) => {
    const device = DEVICES[index];
    const isSource = index === source;
    const status =
      phase === "idle"
        ? "In sync"
        : isSource
          ? phase === "copied"
            ? "Copied"
            : "Sent"
          : phase === "copied"
            ? "Syncing"
            : "Received";

    return (
      <StaggerItem blur={false}>
        <div className="rounded-2xl border border-white/25 bg-linear-to-b from-white/20 to-white/[0.06] p-4 text-left text-white shadow-[inset_0_1px_0_rgb(255_255_255/0.3),0_16px_40px_-12px_rgb(30_20_120/0.5)] backdrop-blur-xl backdrop-saturate-150">
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <p className="truncate text-sm font-medium">{device.name}</p>
              <p className="truncate text-xs text-white/60">{device.os}</p>
            </div>
            <Status label={status} live={status === "Received"} />
          </div>

          <p className="mt-4 text-[11px] font-medium tracking-wide text-white/50 uppercase">
            Clipboard
          </p>
          <div className="mt-1.5 h-6">
            <ClipText step={step} isSource={isSource} phase={phase} />
          </div>
        </div>
      </StaggerItem>
    );
  };

  // DOM order matches the entrance order: card, lock, card.
  return (
    <div
      ref={ref}
      className="grid grid-cols-[minmax(0,1fr)] items-center gap-3 sm:grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] sm:gap-4"
    >
      <LayoutGroup>
        {card(0)}
        <StaggerItem blur={false}>
          <div
            aria-hidden
            className="flex items-center justify-center gap-2 text-white sm:flex-col"
          >
            <span className="h-4 w-px bg-current opacity-50 sm:h-px sm:w-6" />
            <LockIcon className="size-3.5" />
            <span className="h-4 w-px bg-current opacity-50 sm:h-px sm:w-6" />
          </div>
        </StaggerItem>
        {card(1)}
      </LayoutGroup>
    </div>
  );
}

function ClipText({ step, isSource, phase }: { step: number; isSource: boolean; phase: Phase }) {
  const className = "truncate font-mono text-sm";

  // The source copies a new clip: it swaps in with a short blur.
  if (isSource && phase === "copied") {
    return (
      <motion.p
        key={`clip-${step}`}
        layoutId={`clip-${step}`}
        layout="position"
        className={className}
        initial={{ opacity: 0, filter: "blur(4px)" }}
        animate={{ opacity: 1, filter: "blur(0px)" }}
        transition={{ duration: 0.4, ease: easeOut }}
      >
        {clipAt(step)}
      </motion.p>
    );
  }

  // Once sent, the same element reappears in the receiving clipboard and
  // animates across from where it was. The source keeps a static copy.
  if (!isSource && phase === "sent") {
    return (
      <motion.p
        key={`clip-${step}`}
        layoutId={`clip-${step}`}
        layout="position"
        className={className}
        initial={false}
        transition={{ layout: { duration: 0.7, ease: easeInOut } }}
      >
        {clipAt(step)}
      </motion.p>
    );
  }

  // Otherwise the device holds the last synced clip, or its own fresh copy.
  const text = isSource && phase === "sent" ? clipAt(step) : clipAt(step - 1);
  return <p className={className}>{text}</p>;
}

function Status({ label, live }: { label: string; live: boolean }) {
  return (
    <span className="flex shrink-0 items-center gap-1.5 text-xs text-white/70">
      <span
        className={`size-1.5 rounded-full transition-[background-color,box-shadow] duration-300 ${
          live ? "bg-emerald-400 shadow-[0_0_6px_1px_rgb(52_211_153/0.6)]" : "bg-white/30"
        }`}
      />
      <motion.span
        key={label}
        initial={{ opacity: 0, filter: "blur(2px)" }}
        animate={{ opacity: 1, filter: "blur(0px)" }}
        transition={{ duration: 0.25, ease: "easeOut" }}
      >
        {label}
      </motion.span>
    </span>
  );
}

function usePageVisible() {
  const [visible, setVisible] = useState(true);
  useEffect(() => {
    const update = () => setVisible(document.visibilityState === "visible");
    document.addEventListener("visibilitychange", update);
    return () => document.removeEventListener("visibilitychange", update);
  }, []);
  return visible;
}
