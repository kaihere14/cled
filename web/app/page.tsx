import Image from "next/image";
import Link from "next/link";
import { DownloadButton } from "./_components/download-button";
import { ChevronIcon, GitHubIcon, LockIcon } from "./_components/icons";
import { FadeIn, MotionProvider, Stagger, StaggerItem } from "./_components/motion";
import { SyncDemo } from "./_components/sync-demo";
import logo from "./logo.png";
import { site } from "./site";

const REPO_URL = site.repo;
const ARCHITECTURE_URL = `${REPO_URL}/blob/main/docs/architecture.md`;
const BUILDING_URL = `${REPO_URL}/tags`;
const CONTRIBUTING_URL = `${REPO_URL}/blob/main/CONTRIBUTING.md`;

// Structured data so search engines can show Cled as an app, not just a page.
const jsonLd = {
  "@context": "https://schema.org",
  "@type": "SoftwareApplication",
  name: site.name,
  description: site.description,
  url: site.url,
  applicationCategory: "UtilitiesApplication",
  operatingSystem: "Windows, macOS, Linux",
  offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
  codeRepository: site.repo,
  isAccessibleForFree: true,
};

const NAV_LINKS = [
  { label: "How it works", href: ARCHITECTURE_URL },
  { label: "Install", href: BUILDING_URL },
  { label: "Contribute", href: CONTRIBUTING_URL },
];

export default function Home() {
  return (
    <MotionProvider>
      <script
        type="application/ld+json"
        // biome-ignore lint/security/noDangerouslySetInnerHtml: JSON-LD must be raw script text; the data is static and "<" is escaped so it cannot close the tag.
        dangerouslySetInnerHTML={{ __html: JSON.stringify(jsonLd).replace(/</g, "\\u003c") }}
      />
      <Stagger className="relative isolate flex min-h-dvh flex-col overflow-hidden bg-[#06061a] text-white">
        <Backdrop />

        <StaggerItem>
          <a
            href={REPO_URL}
            className="group flex h-11 items-center justify-center gap-2.5 border-b border-white/10 px-5 text-[13px] text-white/70 transition-colors duration-200 hover:text-white"
          >
            <span className="size-1.5 shrink-0 rounded-full bg-indigo-400 shadow-[0_0_8px_2px_rgb(129_140_248/0.6)]" />
            <span className="truncate">Cled is in early development. Follow along on GitHub</span>
            <ChevronIcon className="nudge size-3.5 shrink-0" />
          </a>
        </StaggerItem>

        {/* Transforms make each item its own stacking context; lift the ones with menus. */}
        <StaggerItem className="relative z-30">
          <header className="border-b border-white/[0.06]">
            <nav className="mx-auto flex h-20 max-w-6xl items-center justify-between px-5 sm:px-8">
              <Link href="/" className="pressable flex items-center gap-2.5 rounded-lg">
                <Image
                  src={logo}
                  alt=""
                  width={30}
                  height={30}
                  className="size-[30px] select-none"
                  preload
                />
                <span className="text-lg font-semibold tracking-tight">Cled</span>
              </Link>

              <ul className="hidden items-center gap-9 text-[15px] text-white/70 md:flex">
                {NAV_LINKS.map((link) => (
                  <li key={link.label}>
                    <a href={link.href} className="transition-colors duration-200 hover:text-white">
                      {link.label}
                    </a>
                  </li>
                ))}
              </ul>

              <div className="flex items-center gap-2">
                <a
                  href={REPO_URL}
                  className="pressable hidden items-center gap-1.5 rounded-full px-3.5 py-2 text-[15px] text-white/70 hover:text-white sm:flex"
                >
                  <GitHubIcon className="size-4" />
                  GitHub
                </a>
                <DownloadButton variant="nav" />
              </div>
            </nav>
          </header>
        </StaggerItem>

        <section className="mx-auto flex w-full max-w-6xl flex-1 flex-col items-center px-5 pt-24 pb-16 text-center sm:px-8 sm:pt-32">
          <h1 className="text-[44px] leading-[1.04] font-medium tracking-[-0.035em] sm:text-7xl">
            <StaggerItem as="span" className="block">
              Copy once.
            </StaggerItem>
            <StaggerItem as="span" className="block text-indigo-100/90">
              Paste anywhere.
            </StaggerItem>
          </h1>

          <StaggerItem>
            <p className="mt-7 max-w-2xl text-base leading-relaxed text-balance text-indigo-100/65 sm:text-lg">
              Open-source clipboard sync for Windows, macOS, and Linux. Copy text or an image on one
              device and paste it on the others, end-to-end encrypted.
            </p>
          </StaggerItem>

          <div className="mt-10 flex flex-wrap items-center justify-center gap-4">
            <StaggerItem blur={false} className="relative z-20">
              <DownloadButton variant="hero" />
            </StaggerItem>
            <StaggerItem blur={false}>
              <a
                href={ARCHITECTURE_URL}
                className="pressable flex h-12 items-center gap-1.5 rounded-full border border-white/15 bg-white/10 pr-5 pl-6 text-[15px] font-medium backdrop-blur-md hover:bg-white/15"
              >
                See how it works
                <ChevronIcon className="nudge size-4 opacity-80" />
              </a>
            </StaggerItem>
          </div>

          <div className="mt-14 w-full max-w-3xl sm:mt-16">
            <SyncDemo />
            <StaggerItem blur={false}>
              <p className="mt-5 flex items-center justify-center gap-1.5 text-xs text-balance text-indigo-950/60">
                <LockIcon className="size-3.5 shrink-0" />
                Clips your password manager marks private never leave the device.
              </p>
            </StaggerItem>
          </div>
        </section>
      </Stagger>
    </MotionProvider>
  );
}

function Backdrop() {
  return (
    <div aria-hidden className="pointer-events-none absolute inset-0 -z-10">
      <div className="hero-gradient absolute inset-0" />
      <FadeIn className="hero-grid absolute inset-0" />
      <FadeIn className="hero-dots absolute inset-0" />
    </div>
  );
}
