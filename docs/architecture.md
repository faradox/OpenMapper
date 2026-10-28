# Architecture

## Big picture

```
 ┌──────────── om-app (eframe / egui) ─────────────────────────────────────┐
 │  menus · toolbar · Layers · Sources · Properties · status   ──► Action  │
 │                                                                  │       │
 │  Editor (canvas): select / marquee / move / corner-pin / snap    ▼       │
 │        │ mutates                                   OpenMapperApp::run_action
 │        ▼                                                                 │
 │  Document { project, history, path }  ── commit_if_changed() ─► undo    │
 │        │ &Project, time                                                  │
 └────────┼─────────────────────────────────────────────────────────────────┘
          ▼
 ┌──── om-render ───────────────┐        ┌──── om-core ─────────────────────┐
 │ Renderer::render(project, t) │ uses   │ Project / Surface / Source        │
 │  per surface: uniforms ──────┼───────►│ anim::evaluate(surface, t)        │
 │  draw 2 tris, WGSL fragment  │        │ geom::Homography (perspective)    │
 │  → output texture (Rgba8)    │        │ diff::propagate_changes           │
 └──────────────┬───────────────┘        │ History (snapshots)               │
                │ registered as egui texture                                 │
                ▼                        └───────────────────────────────────┘
   editor preview (zoom/pan)   +   projector window (full screen, stretched)
```

## Frame loop (`OpenMapperApp::ui`)

1. Advance the transport clock (if playing).
2. Read shortcuts and dropped files → queue `Action`s.
3. `Renderer::render(project, time)`: one render pass into the output texture.
4. Lay out the panels and canvas. UI code edits `doc.project` directly.
5. Show the projector viewport (an immediate egui viewport, i.e. a second OS window).
6. Run queued actions.
7. If no pointer button is down, `commit_if_changed()` records an undo snapshot.
8. Update window title, handle close (asks to save), request repaint while playing.

Rendering happens before the UI is drawn, so the preview is at most one frame
behind an edit. That's invisible in practice and keeps the loop simple.

## Rendering

Each visible surface is one draw call of 6 vertices (no vertex buffer). Corners
come from a uniform; the vertex shader converts canvas pixels to clip space.

The fragment shader:

1. Maps the pixel back into the surface's unit square with the **inverse homography**
   (`geom::Homography`, Heckbert's square-to-quad). This gives perspective-correct
   texturing for any four-corner shape. Interpolating UVs across the two triangles
   would not.
2. Computes coverage from the **mask** (quad/ellipse/triangle) using a signed
   distance with `fwidth`-based antialiasing plus optional **feather**.
3. Chooses the base UV (surface UV for *Fit*, canvas UV for *Span*) and applies the
   animated **UV matrix** (offset/scale/rotation/flip + scroll/spin/zoom), with
   aspect correction so rotation stays rigid on non-square surfaces.
4. Samples the source: image texture (repeat addressing), solid color, or a
   procedural **generator** evaluated in the same UV space, so all motion
   controls apply to patterns too.
5. Hue rotation, tint, opacity (with opacity LFO), then premultiplied output.
   Blend modes are separate pipelines with different blend states.

Uniforms for all surfaces go into one buffer with dynamic offsets (stride = 256).

Image sources are decoded with the `image` crate on the UI thread when first seen
or when their path changes. That's fine for v0.1, but video needs a decoder thread
(see roadmap).

## Editor interaction

`editor.rs` holds selection (ordered; last = primary), view (zoom/pan, auto-fit),
and the drag state machine: `Pan | Move | Corner | Marquee`.

- **Hit testing** is front-to-back on unlocked, visible surfaces (point-in-polygon).
- **Corner handles** exist only on selected surfaces and take priority.
- **Snapping**: candidates are corners of non-moving surfaces plus the canvas corners
  and center, within 10 screen px.
- **Welding**: corners within 0.5 px of the dragged corner (on any unlocked surface)
  move with it. Arrow-key nudging respects welding too. Alt/Option bypasses both.

## Multi-selection editing

`panels/properties.rs` clones the primary surface, shows its controls, and if
anything changed calls `diff::propagate_changes(before, after, other)` for every
other selected surface. The diff runs on `serde_json::Value` trees: objects
recurse, leaves (numbers, arrays such as colors or vectors, enum variants) are replaced.
Result: changing *Opacity* on 12 surfaces leaves their different *Time offsets* alone.

Operations that need different values per surface (e.g. **Stagger**) are explicit
`Document` methods.

## Persistence

- Project files: pretty JSON, extension `.omap`, `format_version` field. Relative
  image paths are resolved against the project file's folder on load.
- App state (window size, last opened project): eframe persistence
  (`~/Library/Application Support/OpenMapper` on macOS).

## Known limitations (v0.1)

- Output window placement is manual (drag to projector, then `F`). No monitor picker yet.
- Images only. No video, live camera, Syphon/Spout/NDI yet.
- Surfaces are four-corner only. No mesh warp, no bezier/polygon masks.
- No timeline or keyframes. Animation is parametric (LFO/rates).
- No mipmaps on image textures, so heavily minified images may shimmer.
