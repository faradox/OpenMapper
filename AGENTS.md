# AGENTS.md

Guidance for anyone, human or coding agent, working on OpenMapper. Read
this first, then [docs/architecture.md](docs/architecture.md).

## Product context

- **Who it's for:** a designer with a UX background and years of Photoshop,
  new to projection mapping. She learns fast. She tried MadMapper (animation
  too limited) and HeavyM (handling many shapes at once is cumbersome).
- **What wins:** rich, *easy* texture animation; effortless multi-shape editing;
  an interface that feels like a pro creative tool (Photoshop conventions:
  layers, blend modes, Cmd+Z/D/[/], Space-to-pan, panels with collapsible sections).
- **How we build:** small, working increments. Each one is shipped to her,
  her feedback goes into [docs/feedback-log.md](docs/feedback-log.md), and that
  decides the next step. Prefer finishing one feature well over starting three.
- **Primary platform: macOS** (Apple Silicon and Intel). Development happens on
  Linux; Windows is a bonus. Never add a Linux-only dependency without a macOS story.

## Workspace

| Crate | Purpose | May depend on |
| --- | --- | --- |
| `om-core` | Document model (`Project`, `Surface`, `Source`), perspective math, animation evaluation, undo history, multi-edit diff. Pure Rust, no GPU/UI. | glam, serde |
| `om-render` | wgpu compositor. Renders a `Project` at time *t* into one `Rgba8Unorm` texture. WGSL in `surface.wgsl`. | om-core, wgpu |
| `om-app` | Binary `openmapper`. eframe/egui UI, canvas editor, projector window, file I/O dialogs. | everything |

Keep logic that can be tested without a window in `om-core` (or in plain functions
in `om-app` with unit tests). GPU code goes in `om-render`, UI code in `om-app`.

## Dev loop

```sh
cargo run                      # debug build (deps are optimized, see Cargo.toml)
cargo run --release            # what users get
cargo fmt                      # rustfmt.toml: max_width 110
cargo clippy --all-targets -- -D warnings
cargo test                     # includes WGSL validation through naga
```

CI ([.github/workflows/ci.yml](.github/workflows/ci.yml)) runs fmt, clippy with
`-D warnings` and tests on Linux. It builds a universal macOS `.app` (via
`scripts/bundle-macos.sh`) and a Windows `.exe`. Keep all of them green.

### Rules on the development machine

- Do not change system state (packages, services, global config). Hand the
  maintainer the commands instead. Editing files inside this repo is fine.
- Do **not** drive the GUI on the maintainer's live desktop with xdotool or take
  full-screen screenshots. It's a tiling WM with private windows. Use the isolated
  Xvfb display instead (no GPU there, so force GL; no WM, so set focus explicitly):

  ```sh
  Xvfb :99 -screen 0 1600x1000x24 &
  DISPLAY=:99 WGPU_BACKEND=gl XDG_DATA_HOME=/tmp/om-xdg cargo run &
  W=$(DISPLAY=:99 xdotool search --name OpenMapper | head -1)
  DISPLAY=:99 xdotool windowmove $W 0 0 windowsize $W 1600 1000 windowfocus --sync $W
  DISPLAY=:99 import -window root shot.png      # then inspect the PNG
  ```

  Stop both processes afterwards. Always set `XDG_DATA_HOME` to a scratch dir so
  smoke tests never touch the real persisted state.

## Library versions and API notes

We are on **egui/eframe 0.37** and **wgpu 30**. Both changed a lot recently, so
read the source in `~/.cargo/registry/src/*/egui-0.37.0/` rather than going by memory:

- `eframe::App::ui(&mut self, ui: &mut egui::Ui, frame)` replaces `update(ctx, frame)`.
- Panels: `egui::Panel::left/right/top/bottom(id).show(ui, ..)` and
  `CentralPanel::no_frame()/default_margins().show(ui, ..)`. All take a `Ui`.
- Menus: `egui::MenuBar::new().ui(ui, ..)`; `Button::shortcut_text`, `Button::right_text`.
- Emoji/icon glyphs need egui's `monochrome_emoji_fonts` feature (enabled in the workspace).
  Not every glyph exists: check visually before using a new one.
- wgpu 30: `PipelineLayoutDescriptor { bind_group_layouts: &[Option<&_>], immediate_size }`,
  `multiview_mask` on pipelines and render passes, `depth_slice` on color attachments.
- Always take `wgpu` from the workspace (must match `egui-wgpu`'s version).
- egui native textures must be `Rgba8Unorm` (that's why the output texture is).

## Core design rules

1. **The output texture is the single source of truth.** The editor preview and the
   projector window both show the same texture. Overlays (outlines, handles) are
   drawn by egui on top and are never part of the projected image unless the user
   enables "Outlines on projector".
2. **Animation is a pure function of time** (`om_core::anim::evaluate`). No hidden
   per-frame state, so scrubbing, preview and output always agree.
3. **Undo is snapshot-based.** UI code mutates `doc.project` freely. The app commits
   an undo step when the project changed and no pointer button is down
   (`Document::commit_if_changed`), so a drag becomes one step. Don't add manual
   checkpoint calls.
4. **Multi-edit is generic.** The properties panel edits a copy of the primary
   surface, then `om_core::diff::propagate_changes` copies *only changed fields*
   to the other selected surfaces. New `Surface` fields work automatically, as long
   as they are serde-serializable.
5. **File compatibility.** Every serialized struct uses `#[serde(default)]`. Adding a
   field is fine; renaming or removing one needs a `FORMAT_VERSION` bump and a
   migration in `om_core::project`.
6. **UI actions go through `app::Action`.** Menus, toolbar, panels and shortcuts push
   actions; `OpenMapperApp::run_action` executes them. Shortcuts live in `app::shortcuts`.
   Use `Modifiers::COMMAND` (Cmd on macOS, Ctrl elsewhere).
7. **Coordinates.** Surface corners are canvas pixels, order TL, TR, BR, BL ↔ UV
   (0,0) (1,0) (1,1) (0,1), v pointing down. Colors are straight-alpha sRGB-encoded
   `[f32; 4]`; the shader outputs premultiplied alpha.

## Common tasks

- **New surface property:** add the field to `Surface` (with a default), use it in
  `om_render::surface_uniforms` + `surface.wgsl` if it affects rendering, and add a
  control in `panels/properties.rs`. Multi-edit and undo come for free.
- **New generator pattern:** add a variant to `Pattern` (and to `Pattern::ALL`, at the
  end, because the shader index is the position), then add a `case` in `generator()`
  in `surface.wgsl`. The `shader_indices_are_stable` test guards existing indices.
- **New animation parameter:** extend `Animation` and apply it in `anim::evaluate`,
  with a unit test.
- **New canvas tool/mode:** follow Free Transform (`transform.rs`): pure math functions
  with unit tests, state on `Editor`, `Drag` variants for its gestures, actions for
  enter/apply/cancel, and apply it automatically when the selection changes.
- **New panel or action:** add an `Action` variant, handle it in `run_action`, and wire
  the menu item, shortcut and button.

## Documentation workflow

- Each feature gets a tracker in `docs/trackers/NNNN-short-name.md` (copy
  `docs/trackers/TEMPLATE.md`): goal, scope, design notes, a task checklist, and how it was verified.
  Update it as you go. It's the hand-off document between sessions.
- Record user feedback in `docs/feedback-log.md` with the date and the decision it led to.
- Update `docs/roadmap.md` when priorities change, and `README.md` when user-visible
  features land.

## Commits

Conventional style: `type(scope): imperative summary` (≤ 72 chars), then a bullet
list of concrete changes, ending with the tests added or run. Types: `feat`, `fix`,
`chore`, `docs`, `test`, `refactor`, `perf`. Scopes are crate or feature names
(`core`, `render`, `editor`, `output`, `anim`, `ci`, …). No trailers.
