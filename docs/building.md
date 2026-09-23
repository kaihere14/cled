# Building installers

The **Build** workflow (`.github/workflows/build.yml`) builds installers for all three platforms
on GitHub's runners and publishes them as a **GitHub Release**.

## Running it

- **Manually:** GitHub → Actions → Build → Run workflow. This publishes a pre-release tagged
  `v<version>-build.<run number>`, e.g. `v0.1.0-build.3`.
- **For a version:** push a tag starting with `v`, e.g. `git tag v0.1.0 && git push origin v0.1.0`.

How it runs:

1. A draft release is created.
2. The three platforms build in parallel, each uploading its installers into the draft.
3. The release is published only if all three builds succeeded. If one fails, the draft stays
   unpublished, and you can delete it on the Releases page.

## Downloading

Open the repository's **Releases** page (or the release link on the right of the repository
home page), pick the newest release, and download from **Assets**:

| System | File |
| --- | --- |
| Windows | `.msi`, or `-setup.exe` |
| macOS (Apple Silicon and Intel) | `.dmg` |
| Ubuntu / Debian | `.deb` |
| Fedora / openSUSE | `.rpm` |
| Any Linux | `.AppImage` |

## Installing unsigned builds

These builds are **not code-signed** yet, so each OS warns before running them. That is expected
for development builds.

- **Windows:** SmartScreen shows "Windows protected your PC". Click **More info → Run anyway**.
  Windows may also ask whether Cled can use networks. Allow it, or devices can't sync.
- **macOS:** Gatekeeper refuses to open it ("cannot be verified" or "is damaged"). After moving
  Cled to Applications, either open **System Settings → Privacy & Security** and click
  **Open Anyway**, or run:
  ```sh
  xattr -dr com.apple.quarantine /Applications/Cled.app
  ```
- **Linux:**
  - `.deb`: `sudo apt install ./Cled_*.deb`
  - `.rpm`: `sudo dnf install ./Cled-*.rpm`
  - `.AppImage`: `chmod +x Cled_*.AppImage && ./Cled_*.AppImage`
  - The tray icon needs a StatusNotifierItem host (KDE, waybar's tray, or GNOME with the
    AppIndicator extension).

## Building locally

`pnpm build` builds installers for the machine you're on only; Tauri can't cross-compile
installers for other operating systems. The output lands in `target/release/bundle/`.
