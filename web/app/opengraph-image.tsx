import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { ImageResponse } from "next/og";
import { site } from "./site";

export const alt = `${site.name} — ${site.tagline}`;
export const size = { width: 1200, height: 630 };
export const contentType = "image/png";

const logo = `data:image/png;base64,${await readFile(join(process.cwd(), "app/icon.png"), "base64")}`;

// Same night-to-violet gradient as the hero.
export default function OpengraphImage() {
  return new ImageResponse(
    <div
      style={{
        width: "100%",
        height: "100%",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        color: "white",
        backgroundImage:
          "linear-gradient(180deg, #06061a 0%, #0c0b36 20%, #1b1882 45%, #3a33dc 68%, #6d67f2 86%, #b9b4ff 100%)",
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 20 }}>
        {/* biome-ignore lint/performance/noImgElement: ImageResponse renders to a PNG and only supports plain img, not next/image. */}
        <img src={logo} width={72} height={72} alt="" style={{ borderRadius: 18 }} />
        <div style={{ fontSize: 48, fontWeight: 600, letterSpacing: "-0.02em" }}>{site.name}</div>
      </div>
      <div
        style={{
          marginTop: 44,
          fontSize: 88,
          fontWeight: 600,
          letterSpacing: "-0.04em",
          lineHeight: 1.05,
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
        }}
      >
        <div>Copy once.</div>
        <div style={{ color: "rgba(224, 231, 255, 0.9)" }}>Paste anywhere.</div>
      </div>
      <div style={{ marginTop: 36, fontSize: 28, color: "rgba(224, 231, 255, 0.75)" }}>
        Open-source clipboard sync for Windows, macOS, and Linux
      </div>
    </div>,
    size,
  );
}
