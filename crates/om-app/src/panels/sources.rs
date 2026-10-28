//! Sources panel: the media library (colors, generated patterns, images,
//! videos) plus an editor for the selected source.

use egui::{Button, RichText, Ui};
use om_core::{Generator, Pattern, SourceId, SourceKind};

use super::panel_header;
use crate::app::{Action, OpenMapperApp};
use crate::theme;
use crate::widgets::{color_edit, grid, prop_label, to_color32};

pub fn sources_panel(ui: &mut Ui, app: &mut OpenMapperApp) {
    panel_header(ui, "Sources");

    ui.horizontal(|ui| {
        if ui.small_button("+ Color").clicked() {
            app.act(Action::AddSource(SourceKind::Solid { color: [1.0, 1.0, 1.0, 1.0] }));
        }
        if ui.small_button("+ Pattern").clicked() {
            app.act(Action::AddSource(SourceKind::Generator(Generator::default())));
        }
        if ui.small_button("+ Media…").on_hover_text("Import images and MP4 videos").clicked() {
            app.act(Action::ImportMedia);
        }
    });
    ui.add_space(2.0);

    let has_sel = !app.editor.selection.surfaces.is_empty();
    let ids: Vec<SourceId> = app.doc.project.sources.iter().map(|s| s.id).collect();
    let used_by_selection: Vec<SourceId> = app
        .editor
        .selection
        .surfaces
        .iter()
        .filter_map(|id| app.doc.project.surface(*id).and_then(|s| s.source))
        .collect();

    egui::ScrollArea::vertical().id_salt("sources_list").max_height(130.0).auto_shrink([false, true]).show(
        ui,
        |ui| {
            for id in ids {
                let Some(src) = app.doc.project.source(id) else { continue };
                let selected = app.editor.selected_source == Some(id);
                let in_use = used_by_selection.contains(&id);
                let broken = app.renderer.source_error(id).is_some();
                let resp = ui
                    .horizontal(|ui| {
                        swatch(ui, &src.kind);
                        let mut text = RichText::new(&src.name);
                        if in_use {
                            text = text.color(theme::ACCENT_SOFT);
                        }
                        let hint = match &src.kind {
                            SourceKind::Generator(g) => g.pattern.label(),
                            other => other.label(),
                        };
                        let hint = if broken {
                            RichText::new(format!("⚠ {hint}")).small().color(theme::WARN)
                        } else {
                            RichText::new(hint).small().weak()
                        };
                        ui.add(
                            Button::selectable(selected, text)
                                .right_text(hint)
                                .truncate()
                                .min_size(egui::vec2(ui.available_width() - 4.0, 0.0)),
                        )
                    })
                    .inner;
                let resp = resp.on_hover_text(if has_sel {
                    "Click to edit · double-click to apply to the selected surfaces"
                } else {
                    "Click to edit"
                });
                if resp.clicked() {
                    app.editor.selected_source = Some(id);
                }
                if resp.double_clicked() && has_sel {
                    let sel = app.editor.selection.surfaces.clone();
                    app.doc.assign_source(&sel, Some(id));
                }
            }
        },
    );

    ui.separator();
    let Some(id) = app.editor.selected_source else {
        ui.label(RichText::new("Select a source to edit it.").weak());
        return;
    };

    ui.horizontal(|ui| {
        if ui
            .add_enabled(has_sel, Button::new("Apply to selection"))
            .on_hover_text("Show this source on all selected surfaces")
            .clicked()
        {
            let sel = app.editor.selection.surfaces.clone();
            app.doc.assign_source(&sel, Some(id));
        }
        if ui.button("🗑").on_hover_text("Delete source").clicked() {
            app.doc.project.remove_source(id);
            app.editor.selected_source = None;
        }
    });

    let error = app.renderer.source_error(id).map(str::to_owned);
    let media_size = app.renderer.source_size(id);
    let duration = app.renderer.video_info(id).map(|v| v.duration);
    let mut replace = false;
    let Some(src) = app.doc.project.source_mut(id) else { return };

    egui::ScrollArea::vertical().id_salt("source_editor").auto_shrink([false, false]).show(ui, |ui| {
        grid(ui, "source_props", |ui| {
            prop_label(ui, "Name", "");
            ui.text_edit_singleline(&mut src.name);
            ui.end_row();

            match &mut src.kind {
                SourceKind::Solid { color } => {
                    prop_label(ui, "Color", "");
                    color_edit(ui, color);
                    ui.end_row();
                }
                SourceKind::Generator(g) => generator_ui(ui, g),
                SourceKind::Image { path } | SourceKind::Video { path } => {
                    prop_label(ui, "File", "");
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    ui.label(name).on_hover_text(path.display().to_string());
                    ui.end_row();
                    prop_label(ui, "", "");
                    replace = ui.button("Replace…").clicked();
                    ui.end_row();
                    if let Some((w, h)) = media_size {
                        prop_label(ui, "Size", "");
                        ui.label(format!("{w} × {h} px"));
                        ui.end_row();
                    }
                    if let Some(d) = duration {
                        prop_label(ui, "Length", "Videos loop and follow the play clock");
                        ui.label(format!("{d:.2} s, looping"));
                        ui.end_row();
                    }
                }
            }
        });
        if let Some(err) = error {
            ui.colored_label(theme::WARN, format!("⚠ {err}"));
        }
    });
    if replace {
        app.act(Action::ReplaceMedia(id));
    }
}

fn generator_ui(ui: &mut Ui, g: &mut Generator) {
    prop_label(ui, "Pattern", "");
    egui::ComboBox::from_id_salt("pattern").selected_text(g.pattern.label()).show_ui(ui, |ui| {
        for p in Pattern::ALL {
            ui.selectable_value(&mut g.pattern, p, p.label());
        }
    });
    ui.end_row();

    prop_label(ui, "Colors", "Color A (background) and B (foreground)");
    ui.horizontal(|ui| {
        color_edit(ui, &mut g.color_a);
        if ui.small_button("⇄").on_hover_text("Swap colors").clicked() {
            std::mem::swap(&mut g.color_a, &mut g.color_b);
        }
        color_edit(ui, &mut g.color_b);
    });
    ui.end_row();

    prop_label(ui, "Scale", "How many repetitions across the surface");
    ui.add(egui::Slider::new(&mut g.scale, 0.25..=64.0).logarithmic(true));
    ui.end_row();

    prop_label(ui, "Speed", "The pattern's own motion (0 = still)");
    ui.add(egui::Slider::new(&mut g.speed, -4.0..=4.0));
    ui.end_row();

    prop_label(ui, "Angle", "");
    ui.add(egui::Slider::new(&mut g.angle, -180.0..=180.0).suffix("°"));
    ui.end_row();
}

/// Tiny visual hint of a source's kind and colors.
fn swatch(ui: &mut Ui, kind: &SourceKind) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
    let p = ui.painter();
    match kind {
        SourceKind::Solid { color } => {
            p.rect_filled(rect, 2.0, to_color32(*color));
        }
        SourceKind::Generator(g) => {
            let (l, r) = rect.split_left_right_at_fraction(0.5);
            p.rect_filled(l, 0.0, to_color32(g.color_a));
            p.rect_filled(r, 0.0, to_color32(g.color_b));
        }
        SourceKind::Image { .. } | SourceKind::Video { .. } => {
            let icon = if matches!(kind, SourceKind::Video { .. }) { "🎬" } else { "🖼" };
            p.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                icon,
                egui::FontId::proportional(12.0),
                ui.visuals().text_color(),
            );
        }
    }
    p.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, egui::Color32::from_gray(90)), egui::StrokeKind::Inside);
}
