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
