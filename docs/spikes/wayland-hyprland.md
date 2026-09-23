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

- ~~M3: report the active backend to the UI.~~ Done.
- ~~M3: repeat this spike on GNOME.~~ Partly done, see [GNOME spike](gnome.md). KDE still unverified.
- M4: decide whether clipboard content should outlive the Cled process.

## Addendum (M2): images and privacy hints

| Check | Result |
| --- | --- |
| Password-marked text (`x-kde-passwordManagerHint`, as KeePassXC sets it) | Detected from the format list in ~2 ms and skipped without reading the text. |
| 2560×1440 screenshot (`grim`, `image/png`) | Read and decoded: ~35 ms in release, ~265 ms in debug. |
| 5000×5000 image | Reported as too large. |
| Writing an image | Other apps see `image/png` with the correct size. |
| Polling cost with the screenshot on the clipboard | ~7.3% of a core (release) with full reads every poll; **~0.1%** after adding the Wayland change token. |

## Addendum (M3): change notifications

| Check | Result |
| --- | --- |
| Backend and detection reported | `Wayland`, `Events` (data-control `selection` events) |
| Copy-to-detection latency (5 copies) | 26–28 ms, of which 25 ms is Cled's deliberate debounce |
| Idle CPU with a 2560×1440 screenshot on the clipboard, 20 s | 0 ticks (was ~0.1% of a core with M2 polling) |
| Forced X11 path (`WAYLAND_DISPLAY` unset) | Backend `XWayland`, `Events`; X11 copies detected via XFixes |
