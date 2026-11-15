//! The balloon about the voice connection, opened from "Voz conectada" in the call panel: the
//! ping of the call, what it means, and how the call is protected.

use egui::{Align2, Color32, CornerRadius, FontId, Frame, Rect, Sense, Stroke, Ui, Vec2};

use crate::app::TelinhaApp;
use crate::icons;
use crate::theme::{self, Palette};

const WIDTH: f32 = 236.0;
const ACTION: Color32 = Color32::from_rgb(0x4e, 0x50, 0x58);
const ACTION_HOVER: Color32 = Color32::from_rgb(0x6d, 0x6f, 0x78);

impl TelinhaApp {
    /// Opens or closes the balloon, as a click on "Voz conectada" does.
    pub fn toggle_voice_popover(&mut self) {
        self.room.popover = !self.room.popover;
        self.room.popover_privacy = false;
    }

    pub(crate) fn show_voice_popover(&mut self, context: &egui::Context) {
        if !self.room.popover {
            return;
        }
        let (Some(voice), Some(panel)) = (self.room.voice.clone(), self.room.panel_rect) else {
            self.room.popover = false;
            return;
        };
        let palette = self.palette;
        let place = format!("{} · direto (P2P)", self.place_name(&voice));
        let readings = self.debug.samples("call");
        let samples: Vec<Option<f32>> = readings.iter().map(|(rtt, _)| *rtt).collect();
        let loss = readings.last().and_then(|(_, loss)| *loss).map_or_else(|| "—".to_owned(), |loss| format!("{loss:.1}%"));
        let mut privacy = self.room.popover_privacy;
        let mut debug = false;
        let mut copy = false;
        let now = context.input(|input| input.time);
        let copied = self.room.popover_copied_at.is_some_and(|at| now - at < 1.5);
        let frame = Frame::new()
            .fill(palette.float)
            .corner_radius(CornerRadius::same(8))
            .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(128) });
        let area = egui::Area::new(egui::Id::unique("voice-popover"))
            .order(egui::Order::Foreground)
            .pivot(Align2::LEFT_BOTTOM)
            .fixed_pos(egui::pos2(panel.left() + 8.0, panel.top() - 8.0))
            .show(context, |ui| {
                frame.show(ui, |ui| {
                    ui.set_width(WIDTH);
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    tabs(ui, &palette, &mut privacy);
                    egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 12, top: 10, bottom: 4 }).show(ui, |ui| {
                        ui.set_width(WIDTH - 24.0);
                        if privacy {
                            note(ui, &palette, "Sua voz, sua câmera e sua tela vão direto do seu computador para o das outras pessoas, protegidas com DTLS-SRTP.");
                            note(ui, &palette, "Mensagens, arquivos e avisos de presença passam por um servidor MQTT público, cifrados com AES-256-GCM. As chaves ficam só nos computadores de quem participa.");
                            return;
                        }
                        graph(ui, &palette, &samples);
                        ui.add_space(10.0);
                        let galley = ui.painter().layout(place.clone(), FontId::new(13.0, theme::bold()), palette.strong, WIDTH - 24.0);
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(WIDTH - 24.0, galley.size().y), Sense::hover());
                        ui.painter().galley(rect.min, galley, palette.strong);
                        ui.add_space(4.0);
                        let pings: Vec<f32> = samples.iter().flatten().copied().collect();
                        let average = if pings.is_empty() { "—".to_owned() } else { format!("{:.0} ms", pings.iter().sum::<f32>() / pings.len() as f32) };
                        let last = samples.last().copied().flatten().map_or_else(|| "—".to_owned(), |ping| format!("{ping:.0} ms"));
                        line(ui, &palette, "Ping médio: ", &average);
                        line(ui, &palette, "Último ping: ", &last);
                        line(ui, &palette, "Taxa de perda de pacotes: ", &loss);
                        note(
                            ui,
                            &palette,
                            "Você pode notar atraso no áudio quando o ping passa de 250 ms. A voz pode soar robótica se a perda de pacotes passar de 10%. Se continuar, saia e entre de novo na chamada.",
                        );
                        ui.add_space(10.0);
                        let (row, _) = ui.allocate_exact_size(Vec2::new(WIDTH - 24.0, 28.0), Sense::hover());
                        let half = (row.width() - 8.0) / 2.0;
                        debug = small_button(ui, Rect::from_min_size(row.min, Vec2::new(half, 28.0)), "Depuração");
                        copy = small_button(ui, Rect::from_min_size(row.min + Vec2::new(half + 8.0, 0.0), Vec2::new(half, 28.0)), if copied { "Copiado" } else { "Copiar registro" });
                    });
                    footer(ui, &palette);
                });
            });
        self.room.popover_privacy = privacy;
        if debug {
            self.room.popover = false;
            self.open_debug();
        }
        if copy {
            context.copy_text(self.debug_log_text());
            self.room.popover_copied_at = Some(now);
        }
        if copied {
            context.request_repaint_after(std::time::Duration::from_millis(300));
        }
        // A click anywhere else closes it, except on the button that opened it, which toggles it.
        let on_status = context.input(|input| input.pointer.interact_pos()).is_some_and(|pos| self.room.status_rect.is_some_and(|rect| rect.contains(pos)));
        if area.response.clicked_elsewhere() && !on_status {
            self.room.popover = false;
        }
    }
}

fn tabs(ui: &mut Ui, palette: &Palette, privacy: &mut bool) {
    let (strip, _) = ui.allocate_exact_size(Vec2::new(WIDTH, 10.0 + 4.0 + 18.0 + 8.0 + 2.0 + 1.0), Sense::hover());
    ui.painter().hline(strip.x_range(), strip.bottom() - 0.5, Stroke::new(1.0, palette.line_soft));
    let mut left = strip.left() + 12.0;
    for (label, is_privacy) in [("Conexão", false), ("Privacidade", true)] {
        let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::new(13.0, theme::semibold()), palette.muted);
        let tab = Rect::from_min_size(egui::pos2(left, strip.top() + 10.0), Vec2::new(galley.size().x, 4.0 + 18.0 + 8.0 + 2.0));
        let response = ui.interact(tab, ui.scope_id().with(("popover-tab", label)), Sense::click());
        let selected = *privacy == is_privacy;
        let color = if selected {
            palette.accent_text
        } else if response.hovered() {
            palette.text
        } else {
            palette.muted
        };
        ui.painter().galley_with_override_text_color(egui::pos2(tab.left(), tab.top() + 4.0 + (18.0 - galley.size().y) / 2.0), galley.clone(), color);
        if selected {
            ui.painter().rect_filled(Rect::from_min_max(egui::pos2(tab.left(), tab.bottom() - 2.0), tab.max), CornerRadius::ZERO, palette.accent_text);
        }
        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            *privacy = is_privacy;
        }
        left = tab.right() + 16.0;
    }
}

/// The ping of the last minutes, with its scale on the right and the times under it.
fn graph(ui: &mut Ui, palette: &Palette, samples: &[Option<f32>]) {
    let times = 18.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(WIDTH - 24.0, 6.0 + 64.0 + times + 2.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(4), palette.rail);
    let plot = Rect::from_min_size(rect.min + Vec2::splat(6.0), Vec2::new(rect.width() - 12.0, 64.0));
    let plot_width = plot.width() - 26.0;
    let peak = samples.iter().flatten().fold(100.0f32, |peak, value| peak.max(*value));
    let scale = (peak / 100.0).ceil() * 100.0;
    for (index, value) in [scale, scale / 2.0, 0.0].into_iter().enumerate() {
        let y = plot.top() + 5.0 + (plot.height() - 10.0) * index as f32 / 2.0;
        ui.painter().hline(plot.left()..=plot.left() + plot_width, y, Stroke::new(1.0, palette.line_soft));
        ui.painter().text(egui::pos2(plot.left() + plot_width + 4.0, y + 3.0), Align2::LEFT_BOTTOM, format!("{}", value.round()), FontId::proportional(10.0), palette.muted);
    }
    let steps = samples.len().saturating_sub(1).max(1) as f32;
    let mut line: Vec<egui::Pos2> = Vec::new();
    for (index, sample) in samples.iter().enumerate() {
        match sample {
            Some(ping) => line.push(egui::pos2(
                plot.left() + plot_width * index as f32 / steps,
                plot.top() + 5.0 + (plot.height() - 10.0) * (1.0 - ping.min(scale) / scale),
            )),
            None => {
                if line.len() > 1 {
                    ui.painter().add(egui::Shape::line(std::mem::take(&mut line), Stroke::new(1.5, palette.accent)));
                }
                line.clear();
            }
        }
    }
    if line.len() > 1 {
        ui.painter().add(egui::Shape::line(line, Stroke::new(1.5, palette.accent)));
    }
}

/// A line of the connection tab, its value in bold.
fn line(ui: &mut Ui, palette: &Palette, name: &str, value: &str) {
    let mut job = egui::text::LayoutJob::default();
    job.append(name, 0.0, egui::TextFormat::simple(FontId::proportional(12.0), palette.text));
    job.append(value, 0.0, egui::TextFormat::simple(FontId::new(12.0, theme::bold()), palette.strong));
    let galley = ui.painter().layout_job(job);
    ui.add_space(2.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(WIDTH - 24.0, galley.size().y), Sense::hover());
    ui.painter().galley(rect.min, galley, palette.text);
    ui.add_space(2.0);
}

fn note(ui: &mut Ui, palette: &Palette, text: &str) {
    ui.add_space(8.0);
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), FontId::proportional(11.0), palette.muted, WIDTH - 24.0);
    job.sections[0].format.line_height = Some(11.0 * 1.45);
    let galley = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(WIDTH - 24.0, galley.size().y), Sense::hover());
    ui.painter().galley(rect.min, galley, palette.muted);
}

fn small_button(ui: &mut Ui, rect: Rect, label: &str) -> bool {
    let response = ui.interact(rect, ui.scope_id().with(("popover-button", rect.left() as i32)), Sense::click());
    ui.painter().rect_filled(rect, CornerRadius::same(4), if response.hovered() { ACTION_HOVER } else { ACTION });
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(13.0), Color32::WHITE);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// "Criptografado de ponta a ponta", on a green tint.
fn footer(ui: &mut Ui, palette: &Palette) {
    let (outer, _) = ui.allocate_exact_size(Vec2::new(WIDTH, 8.0 + 6.0 + 15.0 + 6.0 + 10.0), Sense::hover());
    let strip = Rect::from_min_max(outer.min + Vec2::new(12.0, 8.0), outer.max - Vec2::new(12.0, 10.0));
    ui.painter().rect_filled(strip, CornerRadius::same(4), Color32::from_rgba_unmultiplied(35, 165, 90, 31));
    icons::LOCK.image(12.0, palette.green).paint_at(ui, Rect::from_min_size(egui::pos2(strip.left() + 8.0, strip.center().y - 6.0), Vec2::splat(12.0)));
    ui.painter().text(egui::pos2(strip.left() + 8.0 + 12.0 + 6.0, strip.center().y), Align2::LEFT_CENTER, "Criptografado de ponta a ponta", FontId::proportional(11.0), palette.green);
}
