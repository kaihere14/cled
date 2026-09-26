"use client";

import { MotionConfig, motion, stagger, type Variants } from "motion/react";
import type { ReactNode } from "react";

// Strong ease-out: starts fast so entrances feel immediate.
export const easeOut = [0.23, 1, 0.32, 1] as const;

const container: Variants = {
  hidden: {},
  show: { transition: { delayChildren: stagger(0.06) } },
};

// Full transform strings keep these on the compositor instead of the main thread.
const item: Variants = {
  hidden: { opacity: 0, transform: "translateY(8px)", filter: "blur(4px)" },
  show: {
    opacity: 1,
    transform: "translateY(0px)",
    filter: "blur(0px)",
    transition: { duration: 0.7, ease: easeOut },
  },
};

// Without blur, for surfaces with a backdrop-filter: animating a filter on top
// of one repaints the blurred backdrop every frame and stutters.
const lift: Variants = {
  hidden: { opacity: 0, transform: "translateY(12px)" },
  show: {
    opacity: 1,
    transform: "translateY(0px)",
    transition: { duration: 0.7, ease: easeOut },
  },
};

/** Honors prefers-reduced-motion: drops transform and layout motion, keeps fades. */
export function MotionProvider({ children }: { children: ReactNode }) {
  return <MotionConfig reducedMotion="user">{children}</MotionConfig>;
}

/** Children marked with <StaggerItem> enter one after another, 60ms apart. */
export function Stagger({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <motion.div className={className} variants={container} initial="hidden" animate="show">
      {children}
    </motion.div>
  );
}

export function StaggerItem({
  children,
  className,
  as = "div",
  blur = true,
}: {
  children: ReactNode;
  className?: string;
  blur?: boolean;
  /** Use "span" inside elements that only allow phrasing content, like headings. */
  as?: "div" | "span";
}) {
  const Component = as === "span" ? motion.span : motion.div;
  return (
    <Component className={className} variants={blur ? item : lift}>
      {children}
    </Component>
  );
}

/** A slow, one-time fade for ambient layers like backgrounds. */
export function FadeIn({ children, className }: { children?: ReactNode; className?: string }) {
  return (
    <motion.div
      className={className}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 1.2, ease: easeOut }}
    >
      {children}
    </motion.div>
  );
}
