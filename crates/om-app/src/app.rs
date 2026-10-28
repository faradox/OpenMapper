//! Application shell: owns the document, renderer and UI state, routes
//! actions from menus / shortcuts / panels, and drives the frame loop.

use std::path::PathBuf;
use std::time::Instant;

use egui::{Key, KeyboardShortcut, Modifiers};
use glam::Vec2;
use om_core::{FILE_EXTENSION, Mask, Project, SourceKind};
use om_render::Renderer;

use crate::document::Document;
use crate::editor::Editor;
use crate::output::OutputWindow;
use crate::panels;

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tga"];
const LAST_PROJECT_KEY: &str = "last_project";

pub mod shortcuts {
    use super::*;
    const CMD: Modifiers = Modifiers::COMMAND;
    const CMD_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);

    pub const NEW: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::N);
    pub const OPEN: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::O);
    pub const SAVE: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::S);
    pub const SAVE_AS: KeyboardShortcut = KeyboardShortcut::new(CMD_SHIFT, Key::S);
    pub const UNDO: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::Z);
    pub const REDO: KeyboardShortcut = KeyboardShortcut::new(CMD_SHIFT, Key::Z);
    pub const SELECT_ALL: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::A);
    pub const DUPLICATE: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::D);
    pub const FORWARD: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::CloseBracket);
    pub const BACKWARD: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::OpenBracket);
    pub const TO_FRONT: KeyboardShortcut = KeyboardShortcut::new(CMD_SHIFT, Key::CloseBracket);
    pub const TO_BACK: KeyboardShortcut = KeyboardShortcut::new(CMD_SHIFT, Key::OpenBracket);
    pub const ZOOM_FIT: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::Num0);
    pub const ZOOM_100: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::Num1);
    pub const ZOOM_IN: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::Equals);
    pub const ZOOM_OUT: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::Minus);
    pub const OUTPUT: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::Enter);
    pub const ADD_QUAD: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::Q);
}

/// Everything the user can trigger from menus, toolbar, panels or keys.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    NewDemo,
    NewEmpty,
    Open,
    Save,
    SaveAs,
    Quit,
    Undo,
    Redo,
    SelectAll,
    Deselect,
    Duplicate,
    Delete,
    Restack(i32),
    RestackExtreme { front: bool },
    AddSurface(Mask),
    Stagger,
    TogglePlay,
    Restart,
    ZoomFit,
    Zoom(f32),
    ZoomBy(f32),
    ToggleOutput,
    AddSource(SourceKind),
    AddImages,
    Nudge(Vec2),
}

pub struct Transport {
    pub playing: bool,
    pub time: f64,
    last_tick: Option<Instant>,
}

impl Transport {
    fn tick(&mut self) {
        let now = Instant::now();
        if self.playing
            && let Some(last) = self.last_tick
        {
            self.time += (now - last).as_secs_f64().min(0.25);
        }
        self.last_tick = Some(now);
    }
}

pub struct OpenMapperApp {
    pub doc: Document,
    pub editor: Editor,
    pub transport: Transport,
    pub output: OutputWindow,
    pub renderer: Renderer,
    pub stagger_step: f32,
    render_state: egui_wgpu::RenderState,
    output_tex: egui::TextureId,
    output_generation: u64,
    actions: Vec<Action>,
    status: Option<(String, Instant)>,
    title: String,
    frame_times: std::collections::VecDeque<f32>,
}

impl OpenMapperApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
        crate::theme::apply(&cc.egui_ctx);
        let render_state = cc
            .wgpu_render_state
            .clone()
            .ok_or("OpenMapper needs the wgpu renderer (no GPU adapter found?)")?;
        let renderer = Renderer::new(&render_state.device, &render_state.queue);
        let output_tex = render_state.renderer.write().register_native_texture(
            &render_state.device,
            &renderer.output().view,
            wgpu::FilterMode::Linear,
        );

        let last: Option<PathBuf> = cc.storage.and_then(|s| eframe::get_value(s, LAST_PROJECT_KEY));
        let doc = match last.filter(|p| p.exists()) {
            Some(path) => match Project::load(&path) {
                Ok(p) => Document::new(p, Some(path)),
                Err(e) => {
                    log::warn!("could not reopen {}: {e:#}", path.display());
                    Document::new(Project::demo(), None)
                }
            },
            None => Document::new(Project::demo(), None),
        };

        Ok(Self {
            doc,
            editor: Editor::default(),
            transport: Transport { playing: true, time: 0.0, last_tick: None },
            output: OutputWindow::default(),
            renderer,
            stagger_step: 0.25,
            render_state,
            output_tex,
            output_generation: 0,
            actions: Vec::new(),
            status: None,
            title: String::new(),
            frame_times: Default::default(),
        })
    }

    pub fn act(&mut self, action: Action) {
        self.actions.push(action);
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    pub fn status(&self) -> Option<&str> {
        self.status.as_ref().filter(|(_, t)| t.elapsed().as_secs_f32() < 4.0).map(|(m, _)| m.as_str())
    }

    pub fn fps(&self) -> f32 {
        let n = self.frame_times.len().max(1) as f32;
        let avg = self.frame_times.iter().sum::<f32>() / n;
        if avg > 0.0 { 1.0 / avg } else { 0.0 }
    }

    // ---- Keyboard ----------------------------------------------------------

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        use shortcuts::*;
        let text_focus = ctx.egui_wants_keyboard_input();
        let mut acts = Vec::new();
        ctx.input_mut(|i| {
            // Order matters: shortcuts with Shift must be checked first.
            let table: &[(KeyboardShortcut, Action)] = &[
                (SAVE_AS, Action::SaveAs),
                (SAVE, Action::Save),
                (OPEN, Action::Open),
                (NEW, Action::NewDemo),
                (OUTPUT, Action::ToggleOutput),
            ];
            for (sc, a) in table {
                if i.consume_shortcut(sc) {
                    acts.push(a.clone());
                }
            }
            if text_focus {
                return;
            }
            // Space belongs to the canvas (pan / play). Swallow the press so
            // it never activates whichever button last had keyboard focus.
            i.consume_key(Modifiers::NONE, Key::Space);
            let table: &[(KeyboardShortcut, Action)] = &[
                (REDO, Action::Redo),
                (UNDO, Action::Undo),
                (SELECT_ALL, Action::SelectAll),
                (DUPLICATE, Action::Duplicate),
                (TO_FRONT, Action::RestackExtreme { front: true }),
                (TO_BACK, Action::RestackExtreme { front: false }),
                (FORWARD, Action::Restack(1)),
                (BACKWARD, Action::Restack(-1)),
                (ZOOM_FIT, Action::ZoomFit),
                (ZOOM_100, Action::Zoom(1.0)),
                (ZOOM_IN, Action::ZoomBy(1.25)),
                (ZOOM_OUT, Action::ZoomBy(0.8)),
                (ADD_QUAD, Action::AddSurface(Mask::Quad)),
            ];
            for (sc, a) in table {
                if i.consume_shortcut(sc) {
                    acts.push(a.clone());
                }
            }
            if i.consume_key(Modifiers::COMMAND, Key::Y) {
                acts.push(Action::Redo);
            }
            if i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace) {
                acts.push(Action::Delete);
            }
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                acts.push(Action::Deselect);
            }
            let step = if i.modifiers.shift { 10.0 } else { 1.0 };
            for (key, dir) in [
                (Key::ArrowLeft, Vec2::NEG_X),
                (Key::ArrowRight, Vec2::X),
                (Key::ArrowUp, Vec2::NEG_Y),
                (Key::ArrowDown, Vec2::Y),
            ] {
                if i.key_pressed(key) {
                    acts.push(Action::Nudge(dir * step));
                }
            }
        });

        // Space: hold to pan, tap to play/pause.
        if !text_focus && ctx.input(|i| i.key_released(Key::Space)) {
            if !self.editor.space_panned {
                acts.push(Action::TogglePlay);
            }
            self.editor.space_panned = false;
        }
        self.actions.extend(acts);
    }

    // ---- Actions -------------------------------------------------------------

    fn run_actions(&mut self, ctx: &egui::Context) {
        for action in std::mem::take(&mut self.actions) {
            self.run_action(ctx, action);
        }
    }

    fn run_action(&mut self, ctx: &egui::Context, action: Action) {
        log::debug!("action: {action:?}");
        let sel = self.editor.selection.surfaces.clone();
        match action {
            Action::NewDemo | Action::NewEmpty => {
                if self.confirm_discard() {
                    let p = if action == Action::NewDemo { Project::demo() } else { Project::empty() };
                    self.doc = Document::new(p, None);
                    self.editor.selection.clear();
                    self.editor.selected_source = None;
                    self.editor.fit();
                }
            }
            Action::Open => {
                if !self.confirm_discard() {
                    return;
                }
                let picked = rfd::FileDialog::new()
                    .set_title("Open Project")
                    .add_filter("OpenMapper project", &[FILE_EXTENSION])
                    .pick_file();
                if let Some(path) = picked {
                    match Project::load(&path) {
                        Ok(p) => {
                            self.doc = Document::new(p, Some(path));
                            self.editor.selection.clear();
                            self.editor.fit();
                            self.set_status("Project opened");
                        }
                        Err(e) => error_dialog("Could not open project", &format!("{e:#}")),
                    }
                }
            }
            Action::Save => {
                self.save(false);
            }
            Action::SaveAs => {
                self.save(true);
            }
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::Undo => {
                if self.doc.undo() {
                    self.set_status("Undo");
                }
            }
            Action::Redo => {
                if self.doc.redo() {
                    self.set_status("Redo");
                }
            }
            Action::SelectAll => {
                let ids = self
                    .doc
                    .project
                    .surfaces
                    .iter()
                    .filter(|s| s.visible && !s.locked)
                    .map(|s| s.id)
                    .collect();
                self.editor.selection.set(ids);
            }
            Action::Deselect => self.editor.selection.clear(),
            Action::Duplicate => {
                let ids = self.doc.duplicate(&sel);
                if !ids.is_empty() {
                    self.editor.selection.set(ids);
                }
            }
            Action::Delete => {
                self.doc.delete(&sel);
                self.editor.selection.clear();
            }
            Action::Restack(dir) => self.doc.restack(&sel, dir),
            Action::RestackExtreme { front } => self.doc.restack_extreme(&sel, front),
            Action::AddSurface(mask) => {
                let id = self.doc.add_surface(mask);
                self.editor.selection.set(vec![id]);
            }
            Action::Stagger => {
                if sel.len() > 1 {
                    self.doc.stagger(&sel, self.stagger_step);
                    self.set_status(format!(
                        "Staggered {} surfaces by {:.2}s (in selection order)",
                        sel.len(),
                        self.stagger_step
                    ));
                }
            }
            Action::TogglePlay => self.transport.playing = !self.transport.playing,
            Action::Restart => self.transport.time = 0.0,
            Action::ZoomFit => self.editor.fit(),
            Action::Zoom(z) => self.editor.set_zoom(&self.doc.project, z),
            Action::ZoomBy(f) => {
                let z = self.editor.zoom * f;
                self.editor.set_zoom(&self.doc.project, z);
            }
            Action::ToggleOutput => self.output.open = !self.output.open,
            Action::AddSource(kind) => {
                let name = match &kind {
                    SourceKind::Generator(g) => g.pattern.label().to_string(),
                    other => other.label().to_string(),
                };
                let id = self.doc.project.add_source(name, kind);
                self.editor.selected_source = Some(id);
            }
            Action::AddImages => {
                let picked = rfd::FileDialog::new()
                    .set_title("Import Images")
                    .add_filter("Images", IMAGE_EXTENSIONS)
                    .pick_files();
                if let Some(paths) = picked {
                    self.import_images(paths);
                }
            }
            Action::Nudge(delta) => self.nudge(delta),
        }
    }

    fn nudge(&mut self, delta: Vec2) {
        let project = &mut self.doc.project;
        if let Some((id, idx)) = self.editor.selection.corner {
            let Some(pos) = project.surface(id).map(|s| s.corners[idx]) else { return };
            // Welded corners move along, same as when dragging.
            for s in project.surfaces.iter_mut().filter(|s| !s.locked) {
                for c in s.corners.iter_mut() {
                    if c.distance(pos) <= 0.5 {
                        *c += delta;
                    }
                }
            }
        } else {
            for id in &self.editor.selection.surfaces {
                if let Some(s) = project.surface_mut(*id).filter(|s| !s.locked) {
                    s.translate(delta);
                }
            }
        }
    }

    pub fn import_images(&mut self, paths: Vec<PathBuf>) {
        let mut last = None;
        for path in paths {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if !IMAGE_EXTENSIONS.contains(&ext.as_str()) {
                continue;
            }
            let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or("Image".into());
            last = Some(self.doc.project.add_source(name, SourceKind::Image { path }));
        }
        if let Some(id) = last {
            self.editor.selected_source = Some(id);
            if !self.editor.selection.surfaces.is_empty() {
                let sel = self.editor.selection.surfaces.clone();
                self.doc.assign_source(&sel, Some(id));
            }
        }
    }

    fn save(&mut self, force_dialog: bool) -> bool {
        let path = match (&self.doc.path, force_dialog) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new()
                .set_title("Save Project")
                .add_filter("OpenMapper project", &[FILE_EXTENSION])
                .set_file_name(format!("{}.{FILE_EXTENSION}", self.doc.display_name()))
                .save_file()
                .map(|p| if p.extension().is_none() { p.with_extension(FILE_EXTENSION) } else { p }),
        };
        let Some(path) = path else { return false };
        self.doc.commit_if_changed();
        match self.doc.save_to(path) {
            Ok(()) => {
                self.set_status("Saved");
                true
            }
            Err(e) => {
                error_dialog("Could not save project", &format!("{e:#}"));
                false
            }
        }
    }

    /// Asks to save unsaved changes. Returns `false` if the user cancelled.
    fn confirm_discard(&mut self) -> bool {
        self.doc.commit_if_changed();
        if !self.doc.dirty {
            return true;
        }
        let answer = rfd::MessageDialog::new()
            .set_title("Unsaved changes")
            .set_description(format!("Save changes to \"{}\" first?", self.doc.display_name()))
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .show();
        match answer {
            rfd::MessageDialogResult::Yes => self.save(false),
            rfd::MessageDialogResult::No => true,
            _ => false,
        }
    }

    // ---- Frame -------------------------------------------------------------

    fn render_output(&mut self) {
        self.renderer.render(&self.doc.project, self.transport.time);
        if self.renderer.output_generation() != self.output_generation {
            self.output_generation = self.renderer.output_generation();
            self.render_state.renderer.write().update_egui_texture_from_wgpu_texture(
                &self.render_state.device,
                &self.renderer.output().view,
                wgpu::FilterMode::Linear,
                self.output_tex,
            );
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title =
            format!("{}{} — OpenMapper", self.doc.display_name(), if self.doc.dirty { " •" } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if dropped.is_empty() {
            return;
        }
        if let Some(project) = dropped.iter().find(|p| p.extension().is_some_and(|e| e == FILE_EXTENSION)) {
            if self.confirm_discard() {
                match Project::load(project) {
                    Ok(p) => {
                        self.doc = Document::new(p, Some(project.clone()));
                        self.editor.selection.clear();
                    }
                    Err(e) => error_dialog("Could not open project", &format!("{e:#}")),
                }
            }
        } else {
            self.import_images(dropped);
        }
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.confirm_discard() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
    }
}

impl eframe::App for OpenMapperApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let dt = ctx.input(|i| i.unstable_dt);
        self.frame_times.push_back(dt);
        if self.frame_times.len() > 60 {
            self.frame_times.pop_front();
        }

        self.transport.tick();
        self.handle_shortcuts(&ctx);
        self.handle_dropped_files(&ctx);
        self.render_output();

        panels::menu_bar(ui, self);
        panels::toolbar(ui, self);
        panels::status_bar(ui, self);
        panels::left_panel(ui, self);
        panels::properties_panel(ui, self);
        egui::CentralPanel::no_frame().show(ui, |ui| {
            self.editor.ui(ui, &mut self.doc.project, self.output_tex);
        });

        self.output.show(&ctx, self.output_tex, &self.doc.project, &self.editor.selection);

        self.run_actions(&ctx);
        self.editor.selection.retain_existing(&self.doc.project);
        if let Some(id) = self.editor.selected_source
            && self.doc.project.source(id).is_none()
        {
            self.editor.selected_source = None;
        }

        // Commit an undo step once the user lets go of the mouse.
        if !ctx.input(|i| i.pointer.any_down()) && !self.editor.is_interacting() {
            self.doc.commit_if_changed();
        }

        self.update_title(&ctx);
        self.handle_close(&ctx);

        if self.transport.playing {
            ctx.request_repaint();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(path) = &self.doc.path {
            eframe::set_value(storage, LAST_PROJECT_KEY, path);
        }
    }
}

pub fn error_dialog(title: &str, message: &str) {
    log::error!("{title}: {message}");
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(title)
        .set_description(message)
        .show();
}
