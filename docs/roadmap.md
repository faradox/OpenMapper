# Roadmap

Ordered by expected value for the primary user. Real feedback reorders this
list. Record it in [feedback-log.md](feedback-log.md) first.

## ✅ v0.1: Foundation (done)

Four-corner perspective surfaces (quad/ellipse/triangle masks), snapping and welded
corners, multi-select with field-level multi-edit, layers, sources (color, 7 GPU
patterns, images), Fit/Span mapping, parametric texture animation (scroll, spin, hue,
zoom/opacity LFOs, presets, stagger), blend modes, feather, projector window, undo,
save/load. Tracker: [trackers/0001-foundation.md](trackers/0001-foundation.md).

## ✅ Free Transform (done)

`Cmd+T` bounding box: scale (Shift proportional, Alt from center), rotate (Shift 15°),
move, Enter/Esc. Tracker: [trackers/0002-free-transform.md](trackers/0002-free-transform.md).

## Next candidates (pick from feedback)

### Output & show workflow
- Monitor picker: send output to a chosen display in one click, remember it per project.
- Multiple outputs (two projectors), with edge blending later.
- "Show mode": hide the editor overlays and lock everything.
- Syphon (macOS) / Spout (Windows) / NDI output and input.

### Media
- **Video playback** (AVFoundation on macOS via a decoder thread; ffmpeg elsewhere),
  loop/ping-pong, per-surface playback speed.
- GIF animation, image sequences.
- Live camera input.
- Mipmaps for images (smooth minification).
- Media thumbnails in the Sources panel.

### Animation (the main differentiator)
- **Timeline with keyframes** for any property (opacity, corners, colors…), with easing
  curves. Clips and cues.
- Tempo: BPM, tap tempo, LFO rates in beats, audio-reactive modulation (FFT bands).
- Modulation matrix: route any LFO/audio band to any parameter.
- Transitions between sources (fade, wipe, dissolve).
- Scenes/cues: snapshot surface states and crossfade between them.

### Geometry
- Polygon surfaces (n points), bezier masks.
- Mesh warp (grid subdivision) for curved objects.
- Groups (Photoshop-style layer folders) with group transforms.
- Numeric entry in the Free Transform readout (type W/H/angle), Distort/Perspective modes.
- Align and distribute.

### Effects
- Per-surface effect stack (blur, edge glow, kaleidoscope, color grading, invert).
- Shader generators editable as WGSL/ISF.

### macOS polish
- Native menu bar (`muda`) and app icon, file associations (`.omap` double-click).
- Notarized, signed builds.
- Trackpad gestures refinement.
