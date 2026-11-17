//! The window to pick what to share, as `share.js` drew it: tabs for programs, whole screens
//! and capture cards, a card for each, and the quality and the sound of the share.

use std::sync::Arc;

use egui::{Align, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, Stroke, Ui, Vec2};
use serde_json::Value;
use telinha_core::engine::{ShareAudio, TargetKind, Targets};
use telinha_core::hub::{ManualRole, ShareChoice, target_command};

use crate::app::TelinhaApp;
use crate::dialogs::{Tone, button, dialog_frame, header};
use crate::icons;
use crate::room_view::Task;
use crate::stream_settings::{AudioChoice, Preset, Quality, Shared, audio_section, quality_section};
use crate::theme::{self, Palette};
use crate::widgets;

const WIDTH: f32 = 780.0;
const CARD_MIN: f32 = 180.0;
const CARD_GAP: f32 = 12.0;
const CARD_HEIGHT: f32 = 2.0 + 8.0 + 84.0 + 6.0 + 17.0 + 6.0 + 15.0 + 8.0 + 2.0;

/// The tabs of the window. The last one picked stays for the next time, as `picker.tab` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    Windows,
    #[default]
    Monitors,
    Devices,
}

impl Tab {
    fn kind(self) -> TargetKind {
        match self {
            Tab::Windows => TargetKind::Window,
            Tab::Monitors => TargetKind::Monitor,
            Tab::Devices => TargetKind::Device,
        }
    }
}

/// Something that can be shared, as a card shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub kind: TargetKind,
    pub handle: u64,
    pub name: String,
    pub meta: String,
    pub pid: u32,
    /// The sound input that comes with a capture card, when it has one.
    pub audio_id: String,
    pub primary: bool,
}

pub struct Picker {
    /// True when a share is already running and the pick only changes what it shows.
    pub switching: bool,
    /// True when the pick is for a connection with codes, not for the call.
    pub manual: bool,
    pub targets: Option<Result<Targets, String>>,
    pub chosen: Option<Choice>,
    pub quality: Quality,
    pub audio: AudioChoice,
    /// The place of the sound input of the card in the list of inputs.
    pub device: usize,
    filter: String,
}

impl Picker {
    /// Takes a new list. A pick that is no longer in it is dropped.
    pub fn receive(&mut self, result: Result<Targets, String>) {
        if let (Some(chosen), Ok(targets)) = (&self.chosen, &result) {
            if !choices(targets, chosen.kind).iter().any(|choice| choice.handle == chosen.handle) {
                self.chosen = None;
            }
        }
        self.targets = Some(result);
    }

    fn inputs(&self) -> Vec<(String, String)> {
        match &self.targets {
            Some(Ok(targets)) => audio_inputs(targets),
            _ => Vec::new(),
        }
    }
}

/// The sound inputs the engine found, as id and name.
pub fn audio_inputs(targets: &Targets) -> Vec<(String, String)> {
    targets
        .audio_inputs
        .iter()
        .filter_map(|input| {
            let id = input.get("id")?.as_str()?.to_owned();
            let name = input.get("name").and_then(Value::as_str).filter(|name| !name.is_empty()).unwrap_or("Entrada sem nome").to_owned();
            Some((id, name))
        })
        .collect()
}

/// The cards of one kind, as `targetCard` named them.
pub fn choices(targets: &Targets, kind: TargetKind) -> Vec<Choice> {
    let values = match kind {
        TargetKind::Monitor => &targets.monitors,
        TargetKind::Window => &targets.windows,
        TargetKind::Device => &targets.devices,
    };
    values
        .iter()
        .filter_map(|value| {
            let text = |name: &str| value.get(name).and_then(Value::as_str).unwrap_or("").to_owned();
            let number = |name: &str| value.get(name).and_then(Value::as_f64).unwrap_or(0.0);
            let fallback = match kind {
                TargetKind::Monitor => "Tela sem nome",
                TargetKind::Window => "(sem título)",
                TargetKind::Device => "Placa de captura",
            };
            let name = Some(text("name")).filter(|name| !name.is_empty()).unwrap_or_else(|| fallback.to_owned());
            let meta = match kind {
                TargetKind::Monitor => format!("{:.0}×{:.0} · {:.0} Hz", number("width"), number("height"), number("refresh_hz").round()),
                TargetKind::Device if !text("audio_name").is_empty() => format!("som: {}", text("audio_name")),
                TargetKind::Device => "placa de captura".to_owned(),
                TargetKind::Window => format!("{:.0}×{:.0}", number("width"), number("height")),
            };
            Some(Choice {
                kind,
                handle: value.get("handle")?.as_u64()?,
                name,
                meta,
                pid: value.get("pid").and_then(Value::as_u64).unwrap_or(0) as u32,
                audio_id: text("audio_id"),
                primary: value.get("primary").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

/// What the sound of a share asks the engine for.
pub fn share_audio(audio: AudioChoice, pid: u32, device: Option<&str>) -> ShareAudio {
    match (audio, device) {
        (AudioChoice::Program, _) if pid > 0 => ShareAudio::Process { pid },
        (AudioChoice::Device, Some(id)) => ShareAudio::Device { id: id.to_owned() },
        (AudioChoice::None, _) => ShareAudio::None,
        _ => ShareAudio::System { exclude_pid: None },
    }
}

/// A tab of the toolbar, as `.tab`.
fn tab_button(ui: &mut Ui, palette: &Palette, label: &str, selected: bool) -> bool {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::new(15.0, theme::semibold()), palette.muted);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(galley.size().x + 24.0, 30.0), Sense::click());
    let (fill, ink) = if selected {
        (palette.selected, palette.strong)
    } else if response.hovered() {
        (palette.hover, palette.text)
    } else {
        (Color32::TRANSPARENT, palette.muted)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, ink);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// The search box of the programs tab, as `.input.input-small`.
fn filter_field(ui: &mut Ui, palette: &Palette, value: &mut String) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(200.0, 28.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(4), palette.rail);
    let inner = rect.shrink2(Vec2::new(8.0, 5.0));
    let response = ui.put(
        inner,
        egui::TextEdit::singleline(value)
            .frame(egui::Frame::new())
            .font(FontId::proportional(13.0))
            .text_color(palette.text)
            .hint_text(RichText::new("Filtrar pelo nome").color(palette.muted))
            .desired_width(inner.width()),
    );
    if response.has_focus() {
        ui.painter().rect_stroke(rect, CornerRadius::same(4), Stroke::new(2.0, palette.accent), egui::StrokeKind::Outside);
    }
}

/// One card of the grid, as `.target-card`. Returns the click and the double click.
fn card(ui: &mut Ui, palette: &Palette, choice: &Choice, width: f32, selected: bool) -> (bool, bool) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, CARD_HEIGHT), Sense::click());
    let fill = if response.hovered() { palette.hover } else { palette.sidebar };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    if selected {
        ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(2.0, palette.accent), egui::StrokeKind::Inside);
    }
    let inner = rect.shrink(10.0);
    let thumb = Rect::from_min_size(inner.min, Vec2::new(inner.width(), 84.0));
    ui.painter().rect_filled(thumb, CornerRadius::same(4), palette.rail);
    let icon = match choice.kind {
        TargetKind::Monitor => icons::TARGET_MONITOR,
        TargetKind::Window => icons::TARGET_WINDOW,
        TargetKind::Device => icons::TARGET_DEVICE,
    };
    icon.image(40.0, if selected { palette.accent } else { palette.muted }).paint_at(ui, Rect::from_center_size(thumb.center(), Vec2::splat(40.0)));

    let name_y = thumb.bottom() + 6.0 + 8.5;
    let mut name_room = inner.width();
    let badge = choice.primary.then(|| ui.painter().layout_no_wrap("PRINCIPAL".to_owned(), FontId::new(10.0, theme::semibold()), Color32::WHITE));
    if let Some(badge) = &badge {
        name_room -= badge.size().x + 12.0 + 6.0;
    }
    let name_width = widgets::paint_text_clipped(ui, egui::pos2(inner.left(), name_y), name_room, &choice.name, FontId::new(13.0, theme::semibold()), palette.text);
    if let Some(badge) = badge {
        let size = Vec2::new(badge.size().x + 12.0, badge.size().y + 2.0);
        let place = Rect::from_min_size(egui::pos2(inner.left() + name_width + 6.0, name_y - size.y / 2.0), size);
        ui.painter().rect_filled(place, CornerRadius::same(8), palette.accent);
        ui.painter().galley(place.center() - badge.size() / 2.0, badge, Color32::WHITE);
    }
    widgets::paint_text_clipped(ui, egui::pos2(inner.left(), name_y + 8.5 + 6.0 + 7.5), inner.width(), &choice.meta, FontId::proportional(12.0), palette.muted);
    let response = response.on_hover_text(&choice.name).on_hover_cursor(egui::CursorIcon::PointingHand);
    (response.clicked(), response.double_clicked())
}

impl TelinhaApp {
    pub(crate) fn open_picker(&mut self, switching: bool, manual: bool) {
        let (quality, audio) = self.room.last_share.unwrap_or_else(|| (self.prefs.share_quality.as_ref().map(Quality::from_json).unwrap_or_default(), AudioChoice::default()));
        self.room.picker = Some(Picker { switching, manual, targets: None, chosen: None, quality, audio, device: 0, filter: String::new() });
        self.list_targets();
    }

    /// Turns the open window to "Personalizada" with every choice showing. For the tests.
    pub fn picker_custom_for_test(&mut self) {
        if let Some(picker) = &mut self.room.picker {
            picker.quality = Quality { preset: Preset::Custom, bitrate: 14, ..Quality::default() };
        }
    }

    /// Opens the window over a made-up list, with the first screen picked. For the tests.
    pub fn picker_for_test(&mut self, switching: bool, targets: Targets) {
        let chosen = choices(&targets, TargetKind::Monitor).into_iter().next();
        self.room.picker = Some(Picker {
            switching,
            manual: false,
            targets: Some(Ok(targets)),
            chosen,
            quality: Quality::default(),
            audio: AudioChoice::Computer,
            device: 0,
            filter: String::new(),
        });
    }

    pub fn close_picker_for_test(&mut self) {
        self.room.picker = None;
    }

    /// Asks the engine for what can be shared, as "Atualizar" does.
    fn list_targets(&mut self) {
        if let Some(picker) = &mut self.room.picker {
            picker.targets = None;
        }
        let hub = Arc::clone(&self.hub);
        let sender = self.room.sender.clone();
        let context = self.context.clone();
        self.runtime.spawn(async move {
            let _ = sender.send(Task::Targets(hub.share_targets().await));
            context.request_repaint();
        });
    }

    pub(crate) fn show_picker(&mut self, context: &egui::Context) {
        self.drain_tasks();
        let palette = self.palette;
        let network = self.room.share_network_kbps;
        let mut tab = self.room.share_tab;
        let Some(picker) = &mut self.room.picker else { return };
        let settings = !picker.switching;
        let (title, confirm_label) = match (picker.switching, picker.manual) {
            (true, _) => ("Trocar o que você está mostrando", "Trocar"),
            (false, true) => ("O que você quer compartilhar?", "Gerar código"),
            (false, false) => ("Transmitir tela", "Transmitir"),
        };
        let inputs = picker.inputs();
        let mut close = false;
        let mut confirm = false;
        let mut refresh = false;
        let response = crate::glass::modal(context, egui::Id::unique("share-picker"), dialog_frame(&palette), |ui| {
            ui.set_width(WIDTH);
            let top = ui.cursor().min.y;
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            if header(ui, &palette, title) {
                close = true;
            }

            // The toolbar: the tabs, the filter of programs and "Atualizar".
            ui.add_space(8.0);
            ui.allocate_ui_with_layout(Vec2::new(WIDTH, 30.0), Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (which, label) in [(Tab::Windows, "Aplicativos"), (Tab::Monitors, "Tela inteira"), (Tab::Devices, "Placas de captura")] {
                    if tab_button(ui, &palette, label, tab == which) {
                        tab = which;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.add_enabled_ui(picker.targets.is_some(), |ui| {
                        if crate::debug_view::ghost_button(ui, &palette, "Atualizar").clicked() {
                            refresh = true;
                        }
                    });
                    if tab == Tab::Windows {
                        filter_field(ui, &palette, &mut picker.filter);
                    }
                });
            });
            ui.add_space(8.0);

            let status = match &picker.targets {
                None => "Procurando telas, programas e placas de captura…".to_owned(),
                Some(Err(error)) => format!("Não consegui listar as telas: {error}"),
                Some(Ok(targets)) if targets.monitors.is_empty() && targets.windows.is_empty() && targets.devices.is_empty() => {
                    "Não encontrei nada para compartilhar nesta máquina.".to_owned()
                }
                Some(Ok(_)) => "Escolha o que mostrar.".to_owned(),
            };
            ui.add(egui::Label::new(RichText::new(status).color(palette.muted).size(13.0)).wrap());
            ui.add_space(13.0);

            // The cards, as `.picker-grid`: columns of at least 180, between 150 and 300 high.
            let filter = picker.filter.trim().to_lowercase();
            let list: Vec<Choice> = match &picker.targets {
                Some(Ok(targets)) => choices(targets, tab.kind())
                    .into_iter()
                    .filter(|choice| tab != Tab::Windows || filter.is_empty() || choice.name.to_lowercase().contains(&filter))
                    .collect(),
                _ => Vec::new(),
            };
            let grid_width = WIDTH - 4.0 - 12.0;
            let columns = (((grid_width + CARD_GAP) / (CARD_MIN + CARD_GAP)).floor() as usize).max(1);
            let card_width = (grid_width - CARD_GAP * (columns - 1) as f32) / columns as f32;
            let rows = list.len().div_ceil(columns);
            let content = if rows == 0 { 20.0 } else { rows as f32 * CARD_HEIGHT + (rows - 1) as f32 * CARD_GAP + 8.0 };
            let (grid, _) = ui.allocate_exact_size(Vec2::new(WIDTH, content.clamp(150.0, 300.0)), Sense::hover());
            let mut picked = None;
            ui.scope_builder(egui::UiBuilder::new().max_rect(grid), |ui| {
                egui::ScrollArea::vertical().id_salt("share-grid").max_height(grid.height()).auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(CARD_GAP, CARD_GAP);
                    ui.add_space(4.0);
                    if list.is_empty() && picker.targets.is_some() {
                        let empty = match tab {
                            Tab::Windows if !filter.is_empty() => "Nenhum programa com esse nome.",
                            Tab::Devices => "Nenhuma placa de captura conectada. Ligue a placa e toque em Atualizar.",
                            _ => "Nada encontrado aqui.",
                        };
                        ui.label(RichText::new(empty).color(palette.muted).size(13.0));
                    }
                    for row in list.chunks(columns) {
                        ui.horizontal(|ui| {
                            ui.add_space(2.0);
                            for choice in row {
                                let selected = picker.chosen.as_ref().is_some_and(|chosen| chosen.kind == choice.kind && chosen.handle == choice.handle);
                                let (clicked, double) = card(ui, &palette, choice, card_width, selected);
                                if clicked || double {
                                    picked = Some((choice.clone(), double));
                                }
                            }
                        });
                    }
                });
            });
            if let Some((choice, double)) = picked {
                // A card that brings its own sound picks it, unless the person wanted silence.
                if choice.kind == TargetKind::Device && !choice.audio_id.is_empty() && picker.audio != AudioChoice::None {
                    picker.audio = AudioChoice::Device;
                    if let Some(place) = inputs.iter().position(|(id, _)| *id == choice.audio_id) {
                        picker.device = place;
                    }
                }
                picker.chosen = Some(choice);
                confirm |= double;
            }

            let process = picker.chosen.as_ref().is_some_and(|choice| choice.kind == TargetKind::Window && choice.pid > 0);
            let device_share = picker.chosen.as_ref().is_some_and(|choice| choice.kind == TargetKind::Device);
            if settings {
                ui.add_space(12.0);
                let (line, _) = ui.allocate_exact_size(Vec2::new(WIDTH, 1.0), Sense::hover());
                ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, palette.line_soft));
                ui.add_space(4.0);
                // The choices scroll when "Personalizada" makes them taller than the window; the
                // buttons below stay in view.
                let room = (context.content_rect().height() - 2.0 * crate::dialogs::DIALOG_MARGIN - (ui.cursor().min.y - top) - 12.0 - 34.0 - 40.0).max(160.0);
                egui::ScrollArea::vertical().id_salt("share-options").max_height(room).min_scrolled_height(room).show(ui, |ui| {
                    quality_section(ui, &palette, &mut picker.quality, WIDTH, 1, network);
                    audio_section(ui, &palette, &mut picker.audio, &mut picker.device, process, device_share, &inputs, WIDTH);
                });
            }

            ui.add_space(12.0);
            ui.allocate_ui_with_layout(Vec2::new(WIDTH, 34.0), Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.add_enabled_ui(picker.chosen.is_some(), |ui| {
                    if button(ui, &palette, confirm_label, Tone::Primary).clicked() {
                        confirm = true;
                    }
                });
                if button(ui, &palette, "Cancelar", Tone::Plain).clicked() {
                    close = true;
                }
            });
        });
        self.room.share_tab = tab;
        if response.should_close() {
            close = true;
        }
        if refresh {
            self.list_targets();
        }
        if confirm {
            self.confirm_picker();
        } else if close {
            self.room.picker = None;
        }
    }

    fn confirm_picker(&mut self) {
        let Some(picker) = self.room.picker.take() else { return };
        let Some(choice) = picker.chosen.clone() else { return };
        let inputs = picker.inputs();
        if picker.switching {
            let command = target_command(choice.kind, choice.handle);
            let sent = if picker.manual { self.hub.manual_command(ManualRole::Share, &command) } else { self.hub.stream_command(&command) };
            if !sent {
                self.notice("A transmissão ainda está começando. Tente trocar de novo em um instante.");
            } else if let Some(shared) = &mut self.room.shared {
                shared.window_pid = if choice.kind == TargetKind::Window { choice.pid } else { 0 };
                shared.device_share = choice.kind == TargetKind::Device;
            }
            if !inputs.is_empty() {
                self.room.audio_inputs = inputs;
            }
            return;
        }

        let process = choice.kind == TargetKind::Window && choice.pid > 0;
        let device_share = choice.kind == TargetKind::Device;
        // A sound that the pick cannot give falls back on the sound of the computer.
        let audio = match picker.audio {
            AudioChoice::Program if !process => AudioChoice::Computer,
            AudioChoice::Device if !device_share || inputs.is_empty() => AudioChoice::Computer,
            audio => audio,
        };
        let device = picker.device.min(inputs.len().saturating_sub(1));
        self.room.last_share = Some((picker.quality, audio));
        self.prefs.share_quality = Some(picker.quality.to_json());
        self.prefs.save();
        self.room.shared = Some(Shared {
            window_pid: if process { choice.pid } else { 0 },
            audio,
            quality: picker.quality,
            manual: picker.manual,
            device_share,
            device,
            pending_quality: picker.quality.preset != Preset::Auto,
        });
        let request = ShareChoice {
            kind: choice.kind,
            handle: choice.handle,
            name: choice.name.clone(),
            audio: share_audio(audio, choice.pid, inputs.get(device).map(|(id, _)| id.as_str())),
            fps: None,
        };
        self.room.audio_inputs = inputs;
        if picker.manual {
            self.start_manual(ManualRole::Share, Some(request));
        } else if let Err(error) = self.hub.go_live(request) {
            self.room.shared = None;
            self.notice(format!("Não consegui transmitir: {error}."));
        }
    }
}
