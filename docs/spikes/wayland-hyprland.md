# Spike: clipboard behavior on Wayland (Hyprland)

**Date:** 2026-09-23
**Environment:** Fedora 44, Hyprland (Wayland, XWayland on `:0`), NVIDIA RTX 5060 (proprietary
driver), `arboard` 3.6.1 with `wayland-data-control`.
**Tool:** `cargo run -p cled-clipboard --example clip -- <watch|read|write>`, driven from a
terminal alongside `wl-copy` / `wl-paste`.

## Questions and results

| # | Question | Result |
| --- | --- | --- |
| 1 | Which backend does arboard pick? | Wayland data-control (`RUST_LOG=arboard=trace` logs "Successfully initialized the Wayland data control clipboard"). |
| 2 | Can a process with no focused window read the clipboard? | **Yes.** Text set by `wl-copy` is read correctly. |
| 3 | Does the polling watcher see changes from other apps? | **Yes.** Each change was reported once. Re-copying identical text was not reported. `a\r\nb` and `a\nb` counted as the same content. |
| 4 | Does text written by Cled persist after the process exits? | **No.** `wl-paste` works while the process is alive and reports "Nothing is copied" after exit. |
| 5 | Does the X11 path via XWayland work for a background process? | **No, not reliably.** With `WAYLAND_DISPLAY` unset, reads returned nothing while Wayland apps had text copied, the watcher saw no Wayland copies, and X11 writes were not visible to Wayland apps. |
| 6 | What does polling cost? | Small text: ~0 CPU over 10 s. 2 MB of text: ~0.12 core-seconds per 10 s (~1.2% of one core). Every poll transfers the full content from the source app. |

## Conclusions

1. **Data-control works on Hyprland.** The M1 approach (arboard plus polling) is viable on
   wlroots-based compositors and, by extension, likely on KDE, which also supports data-control.
   KDE still needs to be verified.
2. **The XWayland fallback is not a real fallback for a background app.** On Hyprland the
   X11/Wayland clipboard bridge does not help a process without a focused X11 window. Expect the
   same or worse on GNOME, which has no data-control protocol. Compositors without data-control
   should get an explicit "limited mode" instead of silently falling back to X11 (M3).
3. **Clipboard ownership ends with the process.** This is fine while Cled runs in the background.
   Whether content should survive Cled quitting (like `wl-copy`, which forks a serving process)
   is a product decision for M4.
4. **Polling reads the full content every tick.** That's acceptable for text but not for large
   images. M3 should switch Wayland to data-control selection events, which announce MIME types
   without transferring data, before image sync becomes common.

## Follow-ups

- M3: report the active backend (data-control, X11, or unsupported) to the UI instead of relying
  on arboard's silent fallback.
- M3: repeat this spike on GNOME and KDE.
- M4: decide whether clipboard content should outlive the Cled process.
