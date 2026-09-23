# Spike: clipboard behavior on GNOME (Wayland without data-control)

**Date:** 2026-09-23
**Environment:** mutter 50.4 (GNOME's compositor) run headless on the Fedora 44 dev machine
(`mutter --headless --wayland --virtual-monitor 1280x720`) with its own Xwayland. Tested with
`cargo run -p cled-clipboard --example clip -- watch|write` plus `xclip` / `wl-copy` /
`wl-paste` pointed at that session.

## Results

| # | Question | Result |
| --- | --- | --- |
| 1 | Does GNOME offer Wayland data-control? | **No.** arboard's data-control init fails; Cled chooses X11 through XWayland and reports backend `XWayland`, limited. |
| 2 | Do X11 change notifications (XFixes) work through GNOME's Xwayland? | **Yes.** Cled reports `change detection: Events`. |
| 3 | Does Cled see copies made by X11 apps? | **Yes.** Two consecutive `xclip` copies were each reported once. |
| 4 | Can X11 apps paste what Cled writes? | **Yes.** |
| 5 | Does Cled see copies made by native Wayland apps (most GNOME apps)? | **Not tested.** A headless session has no keyboard, and Wayland apps can only copy with keyboard focus there. |
| 6 | Can native Wayland apps paste what Cled writes? | **Not tested**, for the same reason. |

## Conclusions

1. Limited mode is correctly detected and reported on GNOME.
2. The X11 half works with live notifications.
3. The important unknown is the bridge between GNOME's Wayland apps and X11 (questions 5 and 6).
   Mutter synchronizes the Wayland and X11 clipboards, but whether that happens while no X11
   window is focused decides whether Cled is useful on GNOME. On Hyprland it did not
   ([Wayland spike](wayland-hyprland.md), question 5).

## Follow-ups

- Repeat questions 5 and 6 in a real GNOME login (a GNOME Boxes VM works) before claiming GNOME
  support.
- KDE Plasma supports data-control and is expected to behave like Hyprland. Unverified.
