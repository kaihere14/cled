/** Shared facts about the site, used by metadata, the sitemap, and the page. */
export const site = {
  name: "Cled",
  tagline: "Copy once. Paste anywhere.",
  description:
    "Cled is open-source clipboard sync for Windows, macOS, and Linux. Copy text or an image on one device and paste it on the others, end-to-end encrypted.",
  // Set NEXT_PUBLIC_SITE_URL to the production origin; social previews need absolute URLs.
  url: process.env.NEXT_PUBLIC_SITE_URL ?? "http://localhost:3000",
  repo: "https://github.com/kaihere14/cled",
} as const;

/** The release the download buttons point at. Bump the tag when a new build ships. */
const RELEASE_TAG = "v0.1.0-build.6";
const RELEASE_ASSETS = `${site.repo}/releases/download/${RELEASE_TAG}`;

export type Os = "macos" | "windows" | "linux";

export type Platform = {
  os: Os;
  name: string;
  /** The first file is the one the main download button hands out. */
  files: readonly { label: string; detail: string; href: string }[];
};

export const release = {
  tag: RELEASE_TAG,
  page: `${site.repo}/releases/tag/${RELEASE_TAG}`,
  platforms: [
    {
      os: "macos",
      name: "macOS",
      files: [
        {
          label: "Universal",
          detail: ".dmg",
          href: `${RELEASE_ASSETS}/Cled_0.1.0_universal.dmg`,
        },
      ],
    },
    {
      os: "windows",
      name: "Windows",
      files: [
        {
          label: "Installer",
          detail: ".exe",
          href: `${RELEASE_ASSETS}/Cled_0.1.0_x64-setup.exe`,
        },
        { label: "MSI", detail: ".msi", href: `${RELEASE_ASSETS}/Cled_0.1.0_x64_en-US.msi` },
      ],
    },
    {
      os: "linux",
      name: "Linux",
      // AppImage runs on any distro, so it leads over .deb and .rpm.
      files: [
        {
          label: "Any distro",
          detail: ".AppImage",
          href: `${RELEASE_ASSETS}/Cled_0.1.0_amd64.AppImage`,
        },
        { label: "Debian, Ubuntu", detail: ".deb", href: `${RELEASE_ASSETS}/Cled_0.1.0_amd64.deb` },
        {
          label: "Fedora, RHEL",
          detail: ".rpm",
          href: `${RELEASE_ASSETS}/Cled-0.1.0-1.x86_64.rpm`,
        },
      ],
    },
  ] satisfies Platform[],
};
