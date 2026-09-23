Development build of Cled. Download the installer for your system from **Assets** below.

| System | File |
| --- | --- |
| Windows | `.msi`, or `-setup.exe` |
| macOS (Apple Silicon and Intel) | `.dmg` |
| Ubuntu / Debian | `.deb` |
| Fedora / openSUSE | `.rpm` |
| Any Linux | `.AppImage` |

These builds are **not code-signed** yet, so your system will warn before opening them:

- **Windows:** "Windows protected your PC" → **More info** → **Run anyway**.
- **macOS:** "cannot be verified" → System Settings → Privacy & Security → **Open Anyway**, or run
  `xattr -dr com.apple.quarantine /Applications/Cled.app`.
- **Linux:** install normally; for the AppImage, `chmod +x` it first.

When your system asks whether Cled may use the network, allow it, or devices can't sync.

More details: [docs/building.md](https://github.com/kaihere14/cled/blob/main/docs/building.md).
