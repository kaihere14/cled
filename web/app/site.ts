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
