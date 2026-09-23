# Building installers

The **Build** workflow (`.github/workflows/build.yml`) produces installers for all three
platforms on GitHub's runners.

## Running it

- **Manually:** GitHub → Actions → Build → Run workflow.
- **Automatically:** push a tag starting with `v`, e.g. `git tag v0.1.0 && git push origin v0.1.0`.

When the run finishes, download the installers from the **Artifacts** section at the bottom of
the run page:

| Artifact | Contents |
| --- | --- |
| `cled-Windows` | `.msi` and `-setup.exe` installers |
| `cled-macOS` | `.dmg` (universal: Apple Silicon and Intel) |
| `cled-Linux` | `.deb` (Debian/Ubuntu), `.rpm` (Fedora/openSUSE), `.AppImage` (any distro) |

Artifacts are zipped by GitHub and kept for 90 days.

## Installing unsigned builds

These builds are **not code-signed** yet, so each OS warns before running them. That is expected
for development builds.

- **Windows:** SmartScreen shows "Windows protected your PC". Click **More info → Run anyway**.
  Windows may also ask whether Cled can use networks once sync exists.
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
