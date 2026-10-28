//! The central canvas: preview of the output plus direct manipulation of
//! surfaces (select, move, corner-pin), with pan/zoom like an image editor.
//!
//! Interaction summary:
//! - Click a surface to select it; Shift/Cmd-click toggles it in the selection.
//! - Drag on empty canvas draws a selection marquee.
//! - Drag a surface to move the whole selection.
//! - Drag a corner handle to pin it. Corners that sit exactly on top of each
//!   other are *welded* and move together (hold Alt/Option to move one alone).
//! - Corners and moves snap to other surfaces' corners and the canvas corners
//!   (hold Alt/Option to disable snapping).
//! - Space-drag or middle-drag pans; scroll pans; pinch / Cmd+scroll zooms.

use egui::{Color32, Key, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2 as EVec2};
use glam::Vec2;
use om_core::geom::{point_in_polygon, quad_center};
use om_core::{Project, SurfaceId};

use crate::theme;

const HANDLE_RADIUS: f32 = 6.0;
const SNAP_DISTANCE_PX: f32 = 10.0;
/// Corners closer than this (canvas px) count as welded.
const WELD_EPSILON: f32 = 0.5;

#[derive(Default)]
pub struct Selection {
    /// Selected surfaces in the order they were selected; last is primary.
    pub surfaces: Vec<SurfaceId>,
    /// The corner last clicked, for keyboard nudging.
    pub corner: Option<(SurfaceId, usize)>,
}

impl Selection {
    pub fn contains(&self, id: SurfaceId) -> bool {
        self.surfaces.contains(&id)
    }

    pub fn primary(&self) -> Option<SurfaceId> {
        self.surfaces.last().copied()
    }

    pub fn set(&mut self, ids: Vec<SurfaceId>) {
        self.surfaces = ids;
        self.corner = None;
    }

    pub fn toggle(&mut self, id: SurfaceId) {
        if let Some(i) = self.surfaces.iter().position(|s| *s == id) {
            self.surfaces.remove(i);
        } else {
            self.surfaces.push(id);
        }
        self.corner = None;
    }

    pub fn clear(&mut self) {
        self.set(Vec::new());
    }

    /// Drops ids that no longer exist (after undo, delete, load...).
    pub fn retain_existing(&mut self, project: &Project) {
        self.surfaces.retain(|id| project.surface(*id).is_some());
        if let Some((id, _)) = self.corner
            && !self.surfaces.contains(&id)
        {
            self.corner = None;
        }
    }
}

enum Drag {
    None,
    Pan { start_pan: EVec2 },
    Move { origin: Vec2, start: Vec<(SurfaceId, [Vec2; 4])> },
    Corner { origin: Vec2, targets: Vec<(SurfaceId, usize, Vec2)> },
    Marquee { start: Pos2, base: Vec<SurfaceId> },
}

pub struct Editor {
    pub selection: Selection,
    pub selected_source: Option<om_core::SourceId>,
    pub zoom: f32,
    pan: EVec2,
    /// Keep the canvas fitted to the view until the user zooms or pans.
    auto_fit: bool,
    drag: Drag,
    pub snapping: bool,
    pub show_outlines: bool,
    /// Space was used for panning during the current press (so releasing it
    /// should not toggle playback).
    pub space_panned: bool,
    last_rect: Rect,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            selection: Selection::default(),
            selected_source: None,
            zoom: 1.0,
            pan: EVec2::ZERO,
            auto_fit: true,
            drag: Drag::None,
            snapping: true,
            show_outlines: true,
            space_panned: false,
            last_rect: Rect::NOTHING,
        }
    }
}

/// Maps between canvas pixels and screen points for one frame.
#[derive(Clone, Copy)]
pub struct ViewXf {
    origin: Pos2,
    zoom: f32,
}

impl ViewXf {
    pub fn to_screen(self, p: Vec2) -> Pos2 {
        self.origin + EVec2::new(p.x, p.y) * self.zoom
    }

    pub fn to_canvas(self, p: Pos2) -> Vec2 {
        let v = (p - self.origin) / self.zoom;
        Vec2::new(v.x, v.y)
    }
}

impl Editor {
    pub fn is_interacting(&self) -> bool {
        !matches!(self.drag, Drag::None)
    }

    pub fn fit(&mut self) {
        self.auto_fit = true;
        self.pan = EVec2::ZERO;
    }

    pub fn set_zoom(&mut self, project: &Project, zoom: f32) {
        self.auto_fit = false;
        let center = self.last_rect.center();
        self.zoom_about(project, center, zoom);
    }

    fn view(&self, rect: Rect, project: &Project) -> ViewXf {
        let size = project.canvas.size();
        let origin = rect.center() + self.pan - EVec2::new(size.x, size.y) * 0.5 * self.zoom;
        ViewXf { origin, zoom: self.zoom }
    }

    fn zoom_about(&mut self, project: &Project, screen: Pos2, new_zoom: f32) {
        let new_zoom = new_zoom.clamp(0.02, 32.0);
        let xf = self.view(self.last_rect, project);
        let anchor = xf.to_canvas(screen);
        let size = project.canvas.size();
        let new_origin = screen - EVec2::new(anchor.x, anchor.y) * new_zoom;
        self.pan = new_origin - self.last_rect.center() + EVec2::new(size.x, size.y) * 0.5 * new_zoom;
        self.zoom = new_zoom;
    }

    /// Draws the canvas and handles all pointer interaction on it.
    pub fn ui(&mut self, ui: &mut Ui, project: &mut Project, output_tex: egui::TextureId) {
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        self.last_rect = rect;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme::WORKSPACE_BG);

        let size = project.canvas.size();
        if self.auto_fit {
            self.zoom = ((rect.width() - 48.0) / size.x).min((rect.height() - 48.0) / size.y).max(0.02);
            self.pan = EVec2::ZERO;
        }

        // ---- Navigation (scroll / pinch / space / middle mouse) ----------
        let (scroll, zoom_delta, space, alt, toggle_mod, hover) = ui.input(|i| {
            (
                i.smooth_scroll_delta(),
                i.zoom_delta(),
                i.key_down(Key::Space),
                i.modifiers.alt,
                i.modifiers.shift || i.modifiers.command,
                i.pointer.hover_pos(),
            )
        });
        let text_focus = ui.ctx().egui_wants_keyboard_input();
        let space = space && !text_focus;
        if let Some(hp) = hover.filter(|p| rect.contains(*p)) {
            if zoom_delta != 1.0 {
                self.auto_fit = false;
                self.zoom_about(project, hp, self.zoom * zoom_delta);
            } else if scroll != EVec2::ZERO {
                self.auto_fit = false;
                self.pan += scroll;
            }
        }

        let xf = self.view(rect, project);
        let canvas_rect = Rect::from_min_max(xf.to_screen(Vec2::ZERO), xf.to_screen(size));

        // ---- Pointer interaction ----------------------------------------
        let press_origin = ui.input(|i| i.pointer.press_origin());
        let snap = self.snapping && !alt;
        let snap_dist = SNAP_DISTANCE_PX / self.zoom;

        if resp.drag_started() {
            let start = press_origin.unwrap_or(rect.center());
            let middle = resp.dragged_by(egui::PointerButton::Middle);
            if space || middle {
                self.drag = Drag::Pan { start_pan: self.pan };
                self.space_panned |= space;
                self.auto_fit = false;
            } else if let Some((id, idx)) = self.hit_corner(project, &xf, start) {
                let origin = project.surface(id).map(|s| s.corners[idx]).unwrap_or_default();
                let mut targets = vec![(id, idx, origin)];
                if !alt {
                    targets.extend(welded_corners(project, origin, id));
                }
                self.selection.corner = Some((id, idx));
                self.drag = Drag::Corner { origin, targets };
            } else if let Some(id) = hit_surface(project, xf.to_canvas(start)) {
                if toggle_mod {
                    if !self.selection.contains(id) {
                        self.selection.surfaces.push(id);
                    }
                } else if !self.selection.contains(id) {
                    self.selection.set(vec![id]);
                }
                self.selection.corner = None;
                let start_corners = self
                    .selection
                    .surfaces
                    .iter()
                    .filter_map(|sid| project.surface(*sid))
                    .filter(|s| !s.locked)
                    .map(|s| (s.id, s.corners))
                    .collect();
                self.drag = Drag::Move { origin: xf.to_canvas(start), start: start_corners };
            } else {
                let base = if toggle_mod { self.selection.surfaces.clone() } else { Vec::new() };
                self.drag = Drag::Marquee { start, base };
            }
        }

        let pointer = resp.interact_pointer_pos().or(hover);
        if resp.dragged() {
            match &mut self.drag {
                Drag::Pan { start_pan } => {
                    if let Some(origin) = press_origin {
                        let cur = pointer.unwrap_or(origin);
                        self.pan = *start_pan + (cur - origin);
                    }
                }
                Drag::Move { origin, start } => {
                    if let Some(p) = pointer {
                        let mut delta = xf.to_canvas(p) - *origin;
                        if snap {
                            let moving: Vec<SurfaceId> = start.iter().map(|(id, _)| *id).collect();
                            let candidates = snap_candidates(project, &moving, None);
                            let mut best: Option<(f32, Vec2)> = None;
                            for (_, corners) in start.iter() {
                                for c in corners {
                                    let moved = *c + delta;
                                    if let Some((d, target)) = nearest(&candidates, moved, snap_dist)
                                        && best.is_none_or(|(bd, _)| d < bd)
                                    {
                                        best = Some((d, target - moved));
                                    }
                                }
                            }
                            if let Some((_, adjust)) = best {
                                delta += adjust;
                            }
                        }
                        for (id, corners) in start.iter() {
                            if let Some(s) = project.surface_mut(*id) {
                                s.corners = corners.map(|c| c + delta);
                            }
                        }
                    }
                }
                Drag::Corner { origin, targets } => {
                    if let Some(p) = pointer {
                        let (id0, idx0, _) = targets[0];
                        let mut pos = *origin + (xf.to_canvas(p) - xf.to_canvas(press_origin.unwrap_or(p)));
                        if snap {
                            let welded: Vec<SurfaceId> = targets.iter().map(|t| t.0).collect();
                            let candidates = snap_candidates(project, &welded, Some((id0, idx0)));
                            if let Some((_, target)) = nearest(&candidates, pos, snap_dist) {
                                pos = target;
                            }
                        }
                        for (id, idx, _) in targets.iter() {
                            if let Some(s) = project.surface_mut(*id)
                                && !s.locked
                            {
                                s.corners[*idx] = pos;
                            }
                        }
                    }
                }
                Drag::Marquee { start, base } => {
                    if let Some(p) = pointer {
                        let r = Rect::from_two_pos(*start, p);
                        let mut ids = base.clone();
                        for s in &project.surfaces {
                            if !s.visible || s.locked || ids.contains(&s.id) {
                                continue;
                            }
                            let pts = s.corners.map(|c| xf.to_screen(c));
                            let center = xf.to_screen(quad_center(s.corners));
                            if r.contains(center) || pts.iter().any(|p| r.contains(*p)) {
                                ids.push(s.id);
                            }
                        }
                        self.selection.set(ids);
                    }
                }
                Drag::None => {}
            }
        }

        if resp.drag_stopped() || !ui.input(|i| i.pointer.any_down()) {
            self.drag = Drag::None;
        }

        if resp.clicked() {
            let p = resp.interact_pointer_pos().unwrap_or(rect.center());
            if let Some((id, idx)) = self.hit_corner(project, &xf, p) {
                self.selection.corner = Some((id, idx));
            } else if let Some(id) = hit_surface(project, xf.to_canvas(p)) {
                if toggle_mod {
                    self.selection.toggle(id);
                } else {
                    self.selection.set(vec![id]);
                }
            } else if !toggle_mod {
                self.selection.clear();
            }
        }

        // ---- Cursor ------------------------------------------------------
        if let Some(hp) = hover.filter(|p| rect.contains(*p)) {
            let icon = if matches!(self.drag, Drag::Pan { .. }) {
                egui::CursorIcon::Grabbing
            } else if space {
                egui::CursorIcon::Grab
            } else if matches!(self.drag, Drag::Corner { .. }) || self.hit_corner(project, &xf, hp).is_some()
            {
                egui::CursorIcon::Crosshair
            } else if matches!(self.drag, Drag::Move { .. })
                || hit_surface(project, xf.to_canvas(hp)).is_some()
            {
                egui::CursorIcon::Move
            } else {
                egui::CursorIcon::Default
            };
            ui.ctx().set_cursor_icon(icon);
        }

        // ---- Drawing -----------------------------------------------------
        painter.rect_filled(canvas_rect.expand(1.0), 0.0, Color32::BLACK);
        painter.image(
            output_tex,
            canvas_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        painter.rect_stroke(canvas_rect, 0.0, Stroke::new(1.0, Color32::from_gray(90)), StrokeKind::Outside);

        let hovered = hover.filter(|p| rect.contains(*p)).and_then(|p| hit_surface(project, xf.to_canvas(p)));
        draw_overlays(&painter, project, &xf, &self.selection, hovered, self.show_outlines, true);

        if let (Drag::Marquee { start, .. }, Some(p)) = (&self.drag, pointer) {
            let r = Rect::from_two_pos(*start, p);
            painter.rect_filled(r, 0.0, theme::ACCENT.linear_multiply(0.12));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, theme::ACCENT), StrokeKind::Inside);
        }
    }

    fn hit_corner(&self, project: &Project, xf: &ViewXf, p: Pos2) -> Option<(SurfaceId, usize)> {
        let mut best: Option<(f32, SurfaceId, usize)> = None;
        for id in self.selection.surfaces.iter().rev() {
            let Some(s) = project.surface(*id).filter(|s| s.visible && !s.locked) else { continue };
            for (i, c) in s.corners.iter().enumerate() {
                let d = xf.to_screen(*c).distance(p);
                if d <= HANDLE_RADIUS + 3.0 && best.is_none_or(|(bd, ..)| d < bd) {
                    best = Some((d, s.id, i));
                }
            }
        }
        best.map(|(_, id, i)| (id, i))
    }
}

/// Front-most visible, unlocked surface under a canvas point.
fn hit_surface(project: &Project, p: Vec2) -> Option<SurfaceId> {
    project
        .surfaces
        .iter()
        .rev()
        .filter(|s| s.visible && !s.locked)
        .find(|s| point_in_polygon(p, &s.corners))
        .map(|s| s.id)
}

/// Corners of other unlocked surfaces sitting exactly on `pos`.
fn welded_corners(project: &Project, pos: Vec2, except: SurfaceId) -> Vec<(SurfaceId, usize, Vec2)> {
    let mut out = Vec::new();
    for s in project.surfaces.iter().filter(|s| s.id != except && !s.locked) {
        for (i, c) in s.corners.iter().enumerate() {
            if c.distance(pos) <= WELD_EPSILON {
                out.push((s.id, i, *c));
            }
        }
    }
    out
}

/// Points things can snap to: corners of visible surfaces not being moved,
/// plus the canvas corners and center.
fn snap_candidates(
    project: &Project,
    moving: &[SurfaceId],
    own_corner: Option<(SurfaceId, usize)>,
) -> Vec<Vec2> {
    let size = project.canvas.size();
    let mut pts = vec![Vec2::ZERO, Vec2::new(size.x, 0.0), size, Vec2::new(0.0, size.y), size * 0.5];
    for s in project.surfaces.iter().filter(|s| s.visible) {
        let moving_surface = moving.contains(&s.id);
        for (i, c) in s.corners.iter().enumerate() {
            // A surface's own other corners are fine to snap to while
            // pinning one corner, but never while moving the whole thing.
            let is_own = own_corner.is_some_and(|(id, idx)| id == s.id && idx == i);
            let same_surface_pin = own_corner.is_some_and(|(id, _)| id == s.id);
            if is_own || (moving_surface && !same_surface_pin) {
                continue;
            }
            pts.push(*c);
        }
    }
    pts
}

fn nearest(candidates: &[Vec2], p: Vec2, max: f32) -> Option<(f32, Vec2)> {
    candidates
        .iter()
        .map(|c| (c.distance(p), *c))
        .filter(|(d, _)| *d <= max && *d > 1e-4)
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

/// Draws surface outlines, names and corner handles. Shared by the editor
/// and (optionally) the projector output.
pub fn draw_overlays(
    painter: &egui::Painter,
    project: &Project,
    xf: &ViewXf,
    selection: &Selection,
    hovered: Option<SurfaceId>,
    show_outlines: bool,
    show_handles: bool,
) {
    for s in project.surfaces.iter().filter(|s| s.visible) {
        let selected = selection.contains(s.id);
        if !show_outlines && !selected {
            continue;
        }
        let pts: Vec<Pos2> = s.corners.iter().map(|c| xf.to_screen(*c)).collect();
        let stroke = if selected {
            Stroke::new(1.5, theme::ACCENT)
        } else if hovered == Some(s.id) {
            Stroke::new(1.5, theme::ACCENT_SOFT)
        } else {
            Stroke::new(1.0, theme::OUTLINE)
        };
        painter.add(Shape::closed_line(pts.clone(), stroke));

        if show_outlines {
            let label_pos = pts[0] + EVec2::new(6.0, 4.0);
            painter.text(
                label_pos,
                egui::Align2::LEFT_TOP,
                &s.name,
                egui::FontId::proportional(11.0),
                if selected { Color32::WHITE } else { Color32::from_gray(170) },
            );
        }
    }

    if !show_handles {
        return;
    }
    for id in &selection.surfaces {
        let Some(s) = project.surface(*id).filter(|s| s.visible) else { continue };
        for (i, c) in s.corners.iter().enumerate() {
            let p = xf.to_screen(*c);
            let active = selection.corner == Some((s.id, i));
            let fill = if s.locked {
                Color32::from_gray(110)
            } else if active {
                theme::ACCENT
            } else {
                Color32::WHITE
            };
            painter.circle(p, HANDLE_RADIUS, fill, Stroke::new(1.5, theme::ACCENT));
        }
    }
}

/// Like [`draw_overlays`] but for a non-uniform stretch (output window).
pub fn draw_output_overlays(painter: &egui::Painter, rect: Rect, project: &Project, selection: &Selection) {
    let size = project.canvas.size();
    let map = |c: Vec2| {
        Pos2::new(rect.min.x + c.x / size.x * rect.width(), rect.min.y + c.y / size.y * rect.height())
    };
    for s in project.surfaces.iter().filter(|s| s.visible) {
        let selected = selection.contains(s.id);
        let pts: Vec<Pos2> = s.corners.iter().map(|c| map(*c)).collect();
        let stroke =
            if selected { Stroke::new(2.0, theme::ACCENT) } else { Stroke::new(1.0, theme::OUTLINE) };
        painter.add(Shape::closed_line(pts.clone(), stroke));
        if selected {
            for (i, p) in pts.iter().enumerate() {
                let active = selection.corner == Some((s.id, i));
                painter.circle(*p, if active { 7.0 } else { 4.0 }, theme::ACCENT, Stroke::NONE);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use om_core::Surface;

    fn two_adjacent() -> Project {
        let mut p = Project::empty();
        let a = p.new_surface_id();
        let b = p.new_surface_id();
        p.surfaces.push(Surface::rect(a, "a", Vec2::new(100.0, 100.0), Vec2::new(100.0, 100.0)));
        p.surfaces.push(Surface::rect(b, "b", Vec2::new(200.0, 100.0), Vec2::new(100.0, 100.0)));
        p
    }

    #[test]
    fn shared_corners_are_welded() {
        let p = two_adjacent();
        let a = p.surfaces[0].id;
        // a's top-right corner sits on b's top-left corner.
        let welded = welded_corners(&p, Vec2::new(200.0, 100.0), a);
        assert_eq!(welded.len(), 1);
        assert_eq!((welded[0].0, welded[0].1), (p.surfaces[1].id, 0));
    }

    #[test]
    fn locked_surfaces_are_not_welded() {
        let mut p = two_adjacent();
        p.surfaces[1].locked = true;
        assert!(welded_corners(&p, Vec2::new(200.0, 100.0), p.surfaces[0].id).is_empty());
    }

    #[test]
    fn snapping_ignores_moving_surfaces_and_finds_neighbors() {
        let p = two_adjacent();
        let a = p.surfaces[0].id;
        let pts = snap_candidates(&p, &[a], None);
        assert!(!pts.contains(&Vec2::new(100.0, 100.0)), "own corner must not be a target");
        assert!(pts.contains(&Vec2::new(300.0, 200.0)));
        let hit = nearest(&pts, Vec2::new(296.0, 203.0), 10.0).unwrap();
        assert_eq!(hit.1, Vec2::new(300.0, 200.0));
    }

    #[test]
    fn hit_test_prefers_front_surface() {
        let mut p = two_adjacent();
        p.surfaces[1].translate(Vec2::new(-50.0, 0.0)); // overlap
        assert_eq!(hit_surface(&p, Vec2::new(175.0, 150.0)), Some(p.surfaces[1].id));
        p.surfaces[1].locked = true;
        assert_eq!(hit_surface(&p, Vec2::new(175.0, 150.0)), Some(p.surfaces[0].id));
    }
}
