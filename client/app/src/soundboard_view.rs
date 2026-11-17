//! The soundboard: short sounds kept on this computer and fired into the call with a click.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, FontId, Frame, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2};
use telinha_voice::soundboard;

use crate::app::{TelinhaApp, menu_header, menu_item};
use crate::dialogs::{Dialog, Purpose};
use crate::icons;
use crate::theme;

#[derive(Default)]
pub struct SoundboardView {
    open: bool,
    /// Each sound decoded once, by its id.
    decoded: HashMap<String, Arc<[i16]>>,
    search: String,
    /// True right after opening, for the search to take the keyboard.
    focus_pending: bool,
    /// The sound just fired and until when its button stays lit.
    playing: Option<(String, f64)>,
    /// A file picked to add, waiting for its name: the kind, the bytes and the decoded sound.
    picked: Option<(&'static str, Vec<u8>, Arc<[i16]>)>,
}

fn mime_of(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "mp3" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "m4a" | "mp4" => "audio/mp4",
        _ => "application/octet-stream",
    }
}

impl TelinhaApp {
    pub fn open_soundboard(&mut self) {
        self.soundboard.open = true;
        self.soundboard.search.clear();
        self.soundboard.focus_pending = true;
    }

    pub fn toggle_soundboard(&mut self) {
        if self.soundboard.open {
            self.close_soundboard();
        } else {
            self.open_soundboard();
        }
    }

    pub fn close_soundboard(&mut self) {
        self.soundboard.open = false;
    }

    /// Plays a sound here and, in a call, to everyone in it. The error is ready to show.
    fn fire_sound(&mut self, id: &str) -> Result<bool, String> {
        let samples = match self.soundboard.decoded.get(id) {
            Some(samples) => Arc::clone(samples),
            None => {
                let data = self.hub.sound_data(id).ok_or("não encontrei esse som")?;
                let samples: Arc<[i16]> = soundboard::decode(&data)?.into();
                self.soundboard.decoded.insert(id.to_owned(), Arc::clone(&samples));
                samples
            }
        };
        let volume = self.prefs.sound_volume;
        let in_call = self.room.play_sound(Arc::clone(&samples), volume);
        // Out of a call the sound still plays here, to try it out. Never in the tests.
        if !in_call && self.room.audible() {
            soundboard::play(samples, volume);
        }
        Ok(in_call)
    }

    fn add_sound_from_file(&mut self) -> Result<(), String> {
        let extensions = ["mp3", "ogg", "oga", "wav", "flac", "m4a"];
        let Some(path) = rfd::FileDialog::new().add_filter("Sons", &extensions).pick_file() else { return Ok(()) };
        let data = std::fs::read(&path).map_err(|_| "não consegui ler esse arquivo".to_owned())?;
        // A file that cannot be decoded here would be a button that does nothing.
        let samples = soundboard::decode(&data)?;
        let file_name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let name = path.file_stem().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "Som".to_owned());
        // The old app asked for the name before keeping the sound, with the file's name filled in.
        self.soundboard.picked = Some((mime_of(&file_name), data, samples.into()));
        self.dialog = Some(Dialog::for_purpose(Purpose::NameSound, &name));
        Ok(())
    }

    /// Keeps the file picked to add under the name written for it.
    pub(crate) fn keep_picked_sound(&mut self, name: &str) -> Result<(), String> {
        if name.is_empty() {
            return Err("Escreva um nome.".to_owned());
        }
        let Some((mime, data, samples)) = self.soundboard.picked.take() else { return Ok(()) };
        match self.hub.add_sound(name, mime, &data) {
            Ok(sound) => {
                self.soundboard.decoded.insert(sound.id, samples);
                Ok(())
            }
            Err(error) => {
                self.soundboard.picked = Some((mime, data, samples));
                Err(error)
            }
        }
    }

    /// The soundboard as `.soundboard` drew it: a balloon of 420 pixels above the call panel
    /// with a search, the volume, a grid of the sounds and a hint.
    pub(crate) fn show_soundboard(&mut self, context: &egui::Context) {
        if !self.soundboard.open {
            return;
        }
        let palette = self.palette;
        let filter = self.soundboard.search.trim().to_lowercase();
        let sounds: Vec<_> = self.hub.sounds().into_iter().filter(|sound| filter.is_empty() || sound.name.to_lowercase().contains(&filter)).collect();
        let now = context.input(|input| input.time);
        let mut fire = None;
        let mut add = false;
        let mut rename = None;
        let mut remove = None;
        let mut volume = self.prefs.sound_volume * 100.0;
        let mut volume_settled = false;
        let anchor = match self.room.panel_rect {
            Some(panel) => egui::pos2(panel.left() + 8.0, panel.top() - 8.0),
            None => egui::pos2(88.0, context.content_rect().bottom() - 60.0),
        };
        // On glass the panel is a pane of glass drawn under what it holds.
        let frame = if palette.glass {
            Frame::new().inner_margin(Margin::same(12))
        } else {
            Frame::new()
                .fill(palette.sidebar)
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::same(10))
                .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(128) })
        };
        let margin = frame.inner_margin;
        let hint = if self.room.voice.is_some() {
            "Clique para tocar para todo mundo na chamada. Botão direito renomeia ou remove."
        } else {
            "Fora de uma chamada o som toca só para você."
        };
        let focus = std::mem::take(&mut self.soundboard.focus_pending);
        let playing = self.soundboard.playing.clone().filter(|(_, until)| now < *until).map(|(id, _)| id);
        let area = egui::Area::new(egui::Id::unique("soundboard"))
            .order(egui::Order::Foreground)
            .pivot(Align2::LEFT_BOTTOM)
            .fixed_pos(anchor)
            .show(context, |ui| {
                frame.show(ui, |ui| crate::glass::panel_contents(ui, margin, 14.0, |ui| {
                    ui.set_width(400.0);
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        let search_width = 400.0 - 10.0 - 20.0 - 6.0 - 80.0;
                        // On glass the field is lighter glass, rounder, as the message box.
                        let (field_fill, field_round) = if palette.glass { (Color32::from_white_alpha(15), 10) } else { (palette.rail, 4) };
                        let field = Frame::new().fill(field_fill).corner_radius(CornerRadius::same(field_round)).inner_margin(Margin::symmetric(10, 7)).show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.soundboard.search)
                                    .frame(Frame::new())
                                    .font(FontId::proportional(14.0))
                                    .text_color(palette.text)
                                    .hint_text(RichText::new("Encontre o som perfeito").color(palette.muted))
                                    .desired_width(search_width - 20.0),
                            )
                        });
                        if field.inner.has_focus() && palette.glass {
                            let rect = field.response.rect;
                            ui.painter().rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, Color32::from_rgba_unmultiplied(109, 114, 255, 179)), egui::StrokeKind::Inside);
                            ui.painter().rect_stroke(rect, CornerRadius::same(10), Stroke::new(3.0, Color32::from_rgba_unmultiplied(109, 114, 255, 51)), egui::StrokeKind::Outside);
                        } else if field.inner.has_focus() {
                            ui.painter().rect_stroke(field.response.rect, CornerRadius::same(4), Stroke::new(2.0, palette.accent), egui::StrokeKind::Outside);
                        } else if palette.glass {
                            ui.painter().rect_stroke(field.response.rect, CornerRadius::same(10), Stroke::new(1.0, crate::glass::EDGE), egui::StrokeKind::Inside);
                        }
                        if focus {
                            field.inner.request_focus();
                        }
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.add(icons::SPEAKER.image(20.0, palette.muted)).on_hover_text("Volume dos sons");
                        ui.spacing_mut().slider_width = 80.0;
                        let slider = ui.add(egui::Slider::new(&mut volume, 0.0..=100.0).show_value(false).trailing_fill(true).handle_shape(egui::style::HandleShape::Circle)).on_hover_text("Volume dos sons");
                        volume_settled = slider.drag_stopped() || (slider.changed() && !slider.dragged());
                    });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.add_space(2.0);
                        ui.label(RichText::new("SEUS SONS").color(palette.muted).size(12.0).family(theme::bold()));
                    });
                    ui.add_space(6.0);
                    let rows = (sounds.len() + 1).div_ceil(3);
                    let grid_height = rows as f32 * 40.0 + rows.saturating_sub(1) as f32 * 8.0;
                    egui::ScrollArea::vertical().max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
                        let (grid, _) = ui.allocate_exact_size(Vec2::new(398.0, grid_height), Sense::hover());
                        let width = (grid.width() - 16.0) / 3.0;
                        let cell = |index: usize| {
                            Rect::from_min_size(grid.min + Vec2::new((index % 3) as f32 * (width + 8.0), (index / 3) as f32 * 48.0), Vec2::new(width, 40.0))
                        };
                        for (index, sound) in sounds.iter().enumerate() {
                            let rect = cell(index);
                            let response = ui.interact(rect, ui.scope_id().with(("sound", &sound.id)), Sense::click());
                            let fill = if playing.as_deref() == Some(sound.id.as_str()) {
                                palette.accent
                            } else if response.hovered() {
                                palette.selected
                            } else {
                                palette.raised
                            };
                            let lit = playing.as_deref() == Some(sound.id.as_str());
                            if palette.glass && lit {
                                crate::glass::accent_fill(ui.painter(), egui::epaint::RectShape::filled(rect, CornerRadius::same(10), Color32::WHITE).into());
                            } else if palette.glass {
                                // A key of glass: white over the panel, with the light edge.
                                let tint = Color32::from_white_alpha(if response.hovered() { 31 } else { 15 });
                                ui.painter().rect_filled(rect, CornerRadius::same(10), tint);
                                ui.painter().rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, crate::glass::EDGE), egui::StrokeKind::Inside);
                            } else {
                                ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
                            }
                            let name = if sound.name.is_empty() { "Som" } else { sound.name.as_str() };
                            sound_label(ui, rect, Some(icons::SPEAKER), name, palette.strong);
                            if response.clicked() {
                                fire = Some(sound.id.clone());
                            }
                            crate::app::context_menu(&response, &palette, |ui| {
                                menu_header(ui, &palette, name);
                                if menu_item(ui, &palette, "Tocar", false) {
                                    fire = Some(sound.id.clone());
                                }
                                if menu_item(ui, &palette, "Renomear", false) {
                                    rename = Some((sound.id.clone(), sound.name.clone()));
                                }
                                if menu_item(ui, &palette, "Remover", true) {
                                    remove = Some(sound.id.clone());
                                }
                            });
                            response.on_hover_text(format!("Tocar {name}")).on_hover_cursor(egui::CursorIcon::PointingHand);
                        }
                        let rect = cell(sounds.len());
                        let response = ui.interact(rect, ui.scope_id().with("sound-add"), Sense::click());
                        let ink = if response.hovered() { palette.strong } else { palette.muted };
                        if response.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(if palette.glass { 10 } else { 6 }), palette.hover);
                        }
                        dashed_rect(ui, rect.shrink(0.5), palette.line_soft);
                        sound_label(ui, rect, None, "Adicionar som", ink);
                        if response.on_hover_text("Adicionar som").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            add = true;
                        }
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_space(2.0);
                        ui.add(egui::Label::new(RichText::new(hint).color(palette.muted).size(12.0)).wrap());
                    });
                }));
            });

        let capital = |text: String| {
            let mut letters = text.chars();
            letters.next().map_or_else(String::new, |first| first.to_uppercase().chain(letters).collect::<String>())
        };
        if let Some(id) = fire {
            match self.fire_sound(&id) {
                Ok(_) => self.soundboard.playing = Some((id, now + 0.6)),
                Err(error) => self.notice(format!("Não consegui tocar esse som: {error}.")),
            }
        }
        if playing.is_some() {
            context.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if add {
            if let Err(error) = self.add_sound_from_file() {
                self.notice(format!("{}.", capital(error)));
            }
        }
        if let Some((id, name)) = rename {
            self.dialog = Some(Dialog::for_purpose(Purpose::RenameSound(id), &name));
        }
        if let Some(id) = remove {
            self.hub.remove_sound(&id);
            self.soundboard.decoded.remove(&id);
        }
        if (volume / 100.0 - self.prefs.sound_volume).abs() > f32::EPSILON {
            self.prefs.sound_volume = (volume / 100.0).clamp(0.0, 1.0);
        }
        if volume_settled {
            self.prefs.save();
        }
        // A click outside closes it, but not one on the button that toggles it, in its own menu
        // or in the window that renames a sound.
        let on_button = context.input(|input| input.pointer.interact_pos()).is_some_and(|pos| self.room.sounds_rect.is_some_and(|rect| rect.contains(pos)));
        if area.response.clicked_elsewhere() && !on_button && !egui::Popup::is_any_open(context) && self.dialog.is_none() {
            self.soundboard.open = false;
        }
        if context.input(|input| input.key_pressed(egui::Key::Escape)) && self.dialog.is_none() {
            self.soundboard.open = false;
        }
    }
}

/// The glyph and the name of a button of the soundboard, centered together and cut to fit.
fn sound_label(ui: &Ui, rect: Rect, icon: Option<icons::Icon>, name: &str, ink: Color32) {
    let glyph = 16.0;
    let room = (rect.width() - 16.0 - glyph - 6.0).max(10.0);
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_owned(), FontId::proportional(13.0), ink);
    job.wrap = egui::text::TextWrapping { max_width: room, max_rows: 1, break_anywhere: true, ..Default::default() };
    let galley = ui.painter().layout_job(job);
    let total = glyph + 6.0 + galley.size().x;
    let left = rect.center().x - total / 2.0;
    let glyph_rect = Rect::from_min_size(egui::pos2(left, rect.center().y - glyph / 2.0), Vec2::splat(glyph));
    match icon {
        Some(icon) => icon.image(glyph, ink).paint_at(ui, glyph_rect),
        None => {
            ui.painter().text(glyph_rect.center(), Align2::CENTER_CENTER, "+", FontId::proportional(13.0), ink);
        }
    }
    ui.painter().galley(egui::pos2(glyph_rect.right() + 6.0, rect.center().y - galley.size().y / 2.0), galley, ink);
}

/// A rounded box drawn with a dashed line, for the button that adds a sound.
fn dashed_rect(ui: &Ui, rect: Rect, color: Color32) {
    let stroke = Stroke::new(1.0, color);
    let r = 6.0;
    let sides = [
        vec![egui::pos2(rect.left() + r, rect.top()), egui::pos2(rect.right() - r, rect.top())],
        vec![egui::pos2(rect.right(), rect.top() + r), egui::pos2(rect.right(), rect.bottom() - r)],
        vec![egui::pos2(rect.right() - r, rect.bottom()), egui::pos2(rect.left() + r, rect.bottom())],
        vec![egui::pos2(rect.left(), rect.bottom() - r), egui::pos2(rect.left(), rect.top() + r)],
    ];
    for side in sides {
        ui.painter().extend(egui::Shape::dashed_line(&side, stroke, 3.0, 3.0));
    }
    for (center, start) in [
        (egui::pos2(rect.left() + r, rect.top() + r), 180.0f32),
        (egui::pos2(rect.right() - r, rect.top() + r), 270.0),
        (egui::pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (egui::pos2(rect.left() + r, rect.bottom() - r), 90.0),
    ] {
        let points: Vec<_> = (0..=4).map(|step| {
            let angle = (start + step as f32 * 22.5).to_radians();
            center + Vec2::new(angle.cos(), angle.sin()) * r
        }).collect();
        ui.painter().add(egui::Shape::line(points, stroke));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_of_a_sound_comes_from_the_end_of_its_name() {
        assert_eq!(mime_of("Risada.MP3"), "audio/mpeg");
        assert_eq!(mime_of("buzina.ogg"), "audio/ogg");
        assert_eq!(mime_of("sem ponto"), "application/octet-stream");
    }
}
