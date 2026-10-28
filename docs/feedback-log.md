# Feedback log

User feedback, newest first. Each entry: date, build/version, what was tried,
what was said (quote where possible), and the decision it led to (link the tracker).

## Template

```
## YYYY-MM-DD: vX.Y (commit abc1234)

Setup: (projector, object, what she tried to make)

What worked:
-

Pain points / wishes (in her words where possible):
-

Decisions:
- → docs/trackers/NNNN-....md
```

## 2026-10-28: v0.1 + Free Transform, first hands-on session

Pain points / wishes (in her words where possible):
- "Mask -> Cutout from other shapes"
- "Save + Save as + Import image are not working right now"
- "Needs import video (mp4)"

Findings:
- Save/Import: all file dialogs were blocking rfd dialogs called inside winit's event
  handler. On macOS that nested modal loop can re-enter winit, which treats it as fatal.
  On the Linux dev box, rfd's default backend needs xdg-desktop-portal or zenity
  (neither installed), so dialogs failed silently.

Decisions:
- Cutouts as a surface flag ("cut out of layers below"), ordered by the layer stack.
- Async file dialogs + in-app prompts on every platform; GTK3 dialogs on Linux.
- H.264 video via OpenH264 (cross-platform, testable on Linux); AVFoundation later.
  → [trackers/0003-cutouts-video-dialogs.md](trackers/0003-cutouts-video-dialogs.md)

## 2026-10-28: v0.1, first question

- "How do I change the size of a shape?" Only corner dragging existed; no way to scale a
  shape (or several) as a whole.

Decisions:
- Build Photoshop-style Free Transform next.
  → [trackers/0002-free-transform.md](trackers/0002-free-transform.md)

## 2026-10-28: background (before v0.1)

- Tried **MadMapper**: animation features not rich enough; specifically missed the
  ability to animate textures.
- Tried **HeavyM**: liked it overall; handling of multiple shapes at once felt cumbersome.
- UX design background, long-time Photoshop user, new to projection mapping, on macOS.

Decisions:
- v0.1 focuses on parametric texture animation on every surface plus field-level
  multi-selection editing, with Photoshop-like conventions.
  → [trackers/0001-foundation.md](trackers/0001-foundation.md)
