//! The numbers of each connection that is open: ping, loss and what the engines report about
//! the picture. For when someone says "it is lagging" and the question is where.

use std::collections::VecDeque;

use egui::{Align, Color32, CornerRadius, FontId, Frame, Layout, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2};
use serde_json::Value;
use telinha_voice::PeerState;

use crate::app::TelinhaApp;
use crate::theme::{self, Palette};

const MAX_SAMPLES: usize = 90;
const MAX_LOG_LINES: usize = 600;

/// One reading of a connection: the round trip in milliseconds and the loss in percent.
type Sample = (Option<f32>, Option<f32>);

struct Source {
    key: String,
    label: String,
    summary: String,
    samples: VecDeque<Sample>,
    rows: Vec<(&'static str, String)>,
    /// The last count of frames and when it was read, to tell frames per second.
    counted: Option<(f64, f64)>,
}

#[derive(Default)]
pub struct DebugView {
    open: bool,
    sources: Vec<Source>,
    selected: Option<String>,
    log: VecDeque<String>,
    copied_at: Option<f64>,
}

fn number(event: &Value, name: &str) -> Option<f64> {
    event.get(name).and_then(Value::as_f64).filter(|value| value.is_finite())
}

/// A number with its unit and a comma, or a dash when the engine did not say it.
fn finite(value: Option<f64>, digits: usize, unit: &str) -> String {
    value.map_or_else(|| "—".to_owned(), |value| format!("{}{unit}", format!("{value:.digits$}").replace('.', ",")))
}

fn count(event: &Value, name: &str) -> String {
    number(event, name).map_or_else(|| "—".to_owned(), |value| format!("{value:.0}"))
}

/// The lines of the panel for one report of an engine, as the one sending or the one watching.
pub fn rows_of(event: &Value, fps: Option<f64>) -> Vec<(&'static str, String)> {
    let loss = number(event, "loss").map(|loss| loss * 100.0);
    if event.get("role").and_then(Value::as_str) == Some("sender") {
        let size = match (number(event, "width"), number(event, "height")) {
            (Some(width), Some(height)) if width > 0.0 => format!("{width:.0}×{height:.0}"),
            _ => "—".to_owned(),
        };
        let mut rows = vec![
            ("Quadros por segundo", finite(fps, 0, "")),
            ("Resolução", size),
            ("Ping", finite(number(event, "rtt_ms"), 0, " ms")),
            ("Perda de pacotes", finite(loss, 2, " %")),
            ("Taxa de envio", finite(number(event, "bitrate_bps").map(|bits| bits / 1e6), 2, " Mbps")),
            ("Captura", finite(number(event, "capture_ms"), 2, " ms")),
            ("Encode", finite(number(event, "encode_ms"), 2, " ms")),
            ("Quadros enviados", count(event, "frames")),
            ("Keyframes", count(event, "keyframes")),
        ];
        if number(event, "viewers").is_some() {
            rows.push(("Assistindo", count(event, "viewers")));
        }
        rows
    } else {
        vec![
            ("Quadros por segundo", finite(fps, 0, "")),
            ("Ping", finite(number(event, "rtt_ms"), 0, " ms")),
            ("Perda de pacotes", finite(loss, 2, " %")),
            ("Atraso de reprodução", finite(number(event, "delay_ms"), 0, " ms")),
            ("Intervalo entre quadros", finite(number(event, "present_interval_ms"), 1, " ms")),
            ("Quadros exibidos", count(event, "presented")),
            ("Quadros recebidos", count(event, "frames")),
            ("Descartados", count(event, "dropped")),
            ("Lacunas", count(event, "gaps")),
            ("Blocos de áudio", count(event, "audio_blocks")),
        ]
    }
}

/// How a connection is doing by its last reading, as `Debug.quality` graded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    Unknown,
    Good,
    Medium,
    Bad,
}

impl DebugView {
    /// Takes a reading made here rather than by an engine, as `Debug.update` did for the call.
    /// `loss` is in percent.
    pub fn update(&mut self, key: &str, label: &str, summary: &str, rtt: Option<f64>, loss: Option<f64>, rows: Vec<(&'static str, String)>) {
        if !self.sources.iter().any(|source| source.key == key) {
            self.sources.push(Source { key: key.to_owned(), label: String::new(), summary: String::new(), samples: VecDeque::new(), rows: Vec::new(), counted: None });
        }
        let Some(source) = self.sources.iter_mut().find(|source| source.key == key) else { return };
        source.label = label.to_owned();
        source.summary = summary.to_owned();
        source.rows = rows;
        source.samples.push_back((rtt.filter(|rtt| rtt.is_finite()).map(|rtt| rtt as f32), loss.filter(|loss| loss.is_finite()).map(|loss| loss as f32)));
        while source.samples.len() > MAX_SAMPLES {
            source.samples.pop_front();
        }
        if self.selected.as_ref().is_none_or(|selected| !self.sources.iter().any(|source| &source.key == selected)) {
            self.selected = Some(key.to_owned());
        }
    }

    /// The readings of one connection, oldest first: round trip in milliseconds, loss in percent.
    pub fn samples(&self, key: &str) -> Vec<(Option<f32>, Option<f32>)> {
        self.sources.iter().find(|source| source.key == key).map(|source| source.samples.iter().copied().collect()).unwrap_or_default()
    }

    pub fn quality(&self, key: &str) -> Quality {
        let Some((Some(rtt), loss)) = self.sources.iter().find(|source| source.key == key).and_then(|source| source.samples.back().copied()) else {
            return Quality::Unknown;
        };
        let loss = loss.unwrap_or(0.0);
        if rtt < 80.0 && loss < 2.0 {
            Quality::Good
        } else if rtt < 200.0 && loss < 8.0 {
            Quality::Medium
        } else {
            Quality::Bad
        }
    }

    pub fn log(&mut self, line: impl Into<String>) {
        self.log.push_back(line.into());
        while self.log.len() > MAX_LOG_LINES {
            self.log.pop_front();
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.sources.retain(|source| source.key != key);
        if self.selected.as_deref() == Some(key) {
            self.selected = self.sources.first().map(|source| source.key.clone());
        }
    }

    /// Takes one event of an engine. Reports of numbers feed the panel, errors go to the log.
    pub fn engine_event(&mut self, key: &str, label: &str, summary: &str, event: &Value, now: f64) {
        let text = |name: &str| event.get(name).and_then(Value::as_str).unwrap_or("");
        match text("event") {
            "stats" => {}
            "error" => {
                self.log(format!("[{label}] erro {}: {} {}", text("stage"), text("status"), text("message")));
                return;
            }
            _ => return,
        }
        if !self.sources.iter().any(|source| source.key == key) {
            self.sources.push(Source { key: key.to_owned(), label: String::new(), summary: String::new(), samples: VecDeque::new(), rows: Vec::new(), counted: None });
        }
        let Some(source) = self.sources.iter_mut().find(|source| source.key == key) else { return };
        let sender = text("role") == "sender";
        let frames = number(event, if sender { "frames" } else { "presented" });
        let fps = match (source.counted, frames) {
            (Some((at, before)), Some(frames)) if now > at => Some(((frames - before) / (now - at)).max(0.0)),
            _ => None,
        };
        source.counted = frames.map(|frames| (now, frames));
        source.label = label.to_owned();
        source.summary = summary.to_owned();
        source.rows = rows_of(event, fps);
        source.samples.push_back((number(event, "rtt_ms").map(|rtt| rtt as f32), number(event, "loss").map(|loss| (loss * 100.0) as f32)));
        while source.samples.len() > MAX_SAMPLES {
            source.samples.pop_front();
        }
        if self.selected.as_ref().is_none_or(|selected| !self.sources.iter().any(|source| &source.key == selected)) {
            self.selected = Some(key.to_owned());
        }
    }
}

/// A graph of the last readings in its box, newest at the right, as `.graph` drew it: the name
/// and the last value on top, the scale on the left.
#[allow(clippy::too_many_arguments)]
fn graph(ui: &mut Ui, palette: &Palette, title: &str, value: &str, samples: &VecDeque<Sample>, pick: fn(&Sample) -> Option<f32>, least_scale: f32, step: f32, ink: Color32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 10.0 + 18.0 + 6.0 + 120.0 + 10.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(6), palette.rail);
    let caption = rect.top() + 10.0 + 9.0;
    ui.painter().text(egui::pos2(rect.left() + 10.0, caption), egui::Align2::LEFT_CENTER, title, FontId::proportional(13.0), palette.muted);
    ui.painter().text(egui::pos2(rect.right() - 10.0, caption), egui::Align2::RIGHT_CENTER, value, FontId::new(13.0, theme::bold()), palette.strong);
    let canvas = Rect::from_min_size(egui::pos2(rect.left() + 10.0, rect.top() + 10.0 + 18.0 + 6.0), Vec2::new(width - 20.0, 120.0));
    let painter = ui.painter().with_clip_rect(canvas);
    let peak = samples.iter().filter_map(pick).fold(least_scale, f32::max);
    let scale = (peak / step).ceil() * step;
    let plot = |value: f32| canvas.top() + 8.0 + (canvas.height() - 16.0) * (1.0 - value.min(scale) / scale);
    for line in 0..=2 {
        let y = canvas.top() + 8.0 + (canvas.height() - 16.0) * line as f32 / 2.0;
        painter.hline(canvas.x_range(), y, Stroke::new(1.0, palette.line_soft));
        let baseline = canvas.top() + (y - canvas.top() - 2.0).max(18.0);
        painter.text(egui::pos2(canvas.left() + 4.0, baseline + 2.0), egui::Align2::LEFT_BOTTOM, format!("{:.0}", scale - scale * line as f32 / 2.0), FontId::proportional(11.0), palette.muted);
    }
    let mut stretch: Vec<egui::Pos2> = Vec::new();
    let flush = |stretch: &mut Vec<egui::Pos2>| {
        if stretch.len() > 1 {
            painter.add(egui::Shape::line(std::mem::take(stretch), Stroke::new(2.0, ink)));
        }
        stretch.clear();
    };
    for (index, sample) in samples.iter().enumerate() {
        match pick(sample) {
            Some(value) => {
                let x = canvas.right() - (samples.len() - 1 - index) as f32 * canvas.width() / (MAX_SAMPLES - 1) as f32;
                stretch.push(egui::pos2(x, plot(value)));
            }
            // A reading without a number breaks the line instead of drawing through it.
            None => flush(&mut stretch),
        }
    }
    flush(&mut stretch);
}

impl TelinhaApp {
    pub fn open_debug(&mut self) {
        self.debug.open = true;
    }

    /// The log of this session, as "Copiar registro" copies it.
    pub(crate) fn debug_log_text(&self) -> String {
        self.debug.log.iter().cloned().collect::<Vec<_>>().join("\n")
    }

    pub fn close_debug(&mut self) {
        self.debug.open = false;
    }

    /// Feeds the panel with what an engine said. `key` names the connection: the share of this
    /// app, the id of who is being watched, or the connection with codes.
    pub(crate) fn debug_engine_event(&mut self, key: &str, event: &Value) {
        let (label, summary) = match key {
            "outgoing" => ("Sua transmissão".to_owned(), "Você está transmitindo a tela na chamada.".to_owned()),
            "manual" => ("Conexão com código".to_owned(), "A conexão feita com convite e resposta.".to_owned()),
            sharer => {
                let name = self.room.streams.incoming.iter().find(|entry| entry.sharer_id == sharer).map_or("alguém", |entry| entry.name.as_str());
                (format!("Tela de {name}"), format!("Você está assistindo a tela de {name}."))
            }
        };
        let now = self.context.input(|input| input.time);
        self.debug.engine_event(key, &label, &summary, event, now);
    }

    /// Feeds the panel a made-up report of the share of this app, one second after the last.
    /// For the tests.
    pub fn debug_for_test(&mut self, event: &Value) {
        let at = self.debug.sources.first().map_or(0.0, |source| source.samples.len() as f64);
        self.debug.engine_event("outgoing", "Sua transmissão", "Você está transmitindo a tela na chamada.", event, at);
    }

    /// The lines about the voice call, read at the moment they are drawn.
    fn call_rows(&self) -> Option<Vec<(&'static str, String)>> {
        let voice = self.room.voice.as_ref()?;
        let others: Vec<_> = self.people_in(&voice.space_id, &voice.room_id).into_iter().filter(|person| !person.me).collect();
        let connected = others.iter().filter(|person| self.room.peer_state(&person.id) == Some(PeerState::Connected)).count();
        // The voice of the call reports no numbers yet, so those wait with a dash as the old
        // app had them before the first reading.
        Some(vec![
            ("Conectados", format!("{connected} de {}", others.len())),
            ("Ping médio", "—".to_owned()),
            ("Perda de pacotes", "—".to_owned()),
            ("Voz reconstruída", "—".to_owned()),
            ("Voz esticada", "—".to_owned()),
            ("Buffer da voz", "—".to_owned()),
            ("Microfone", if voice.mic && !voice.deaf { "ligado" } else { "mudo" }.to_owned()),
            ("Câmera", "desligada".to_owned()),
        ])
    }

    pub(crate) fn show_debug(&mut self, context: &egui::Context) {
        if !self.debug.open {
            return;
        }
        let palette = self.palette;
        let call_rows = self.call_rows();
        let call_summary = self.room.voice.as_ref().map(|voice| format!("{} · voz e câmera direto entre os computadores", self.place_name(voice)));
        let now = context.input(|input| input.time);
        let mut close = false;
        let mut select = None;
        let mut copy = false;
        // The numbers move while the window is open.
        context.request_repaint_after(std::time::Duration::from_millis(500));
        let width = (context.content_rect().width() - 48.0).min(720.0);
        let frame = Frame::new()
            .fill(palette.main)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(20))
            .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(128) });
        let response = crate::glass::modal(context, egui::Id::unique("debug"), frame, |ui| {
            let inner = width - 40.0;
            ui.set_width(inner);
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.horizontal(|ui| {
                ui.label(RichText::new("Depuração da conexão").color(palette.strong).size(18.0).family(theme::bold()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    close = close_button(ui, &palette);
                });
            });
            ui.add_space(8.0);

            // The call comes first, as it is the first connection made on entering a room.
            let selected = self.debug.selected.clone().filter(|key| key == "call" && call_rows.is_some() || self.debug.sources.iter().any(|source| &source.key == key));
            let selected = selected.or_else(|| call_rows.as_ref().map(|_| "call".to_owned())).or_else(|| self.debug.sources.first().map(|source| source.key.clone()));
            let mut tabs: Vec<(String, String)> = Vec::new();
            if call_rows.is_some() {
                tabs.push(("call".to_owned(), "Chamada".to_owned()));
            }
            tabs.extend(self.debug.sources.iter().map(|source| (source.key.clone(), source.label.clone())));
            if !tabs.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(4.0);
                    for (key, label) in &tabs {
                        if tab(ui, &palette, label, selected.as_deref() == Some(key.as_str())) {
                            select = Some(key.clone());
                        }
                    }
                });
                ui.add_space(6.0);
            }

            let source = selected.as_ref().and_then(|key| self.debug.sources.iter().find(|source| &source.key == key));
            let empty = VecDeque::new();
            let (summary, samples, rows) = match (source, &call_rows, selected.as_deref()) {
                (Some(source), _, _) => (source.summary.clone(), &source.samples, source.rows.clone()),
                (None, Some(rows), Some("call")) => (call_summary.clone().unwrap_or_default(), &empty, rows.clone()),
                _ => ("Nada conectado agora. Entre numa chamada ou transmissão para ver os números.".to_owned(), &empty, Vec::new()),
            };
            ui.add_space(13.0);
            ui.add(egui::Label::new(RichText::new(summary).color(palette.muted).size(13.0)).wrap());
            ui.add_space(13.0);

            let last = samples.back().copied().unwrap_or((None, None));
            let pings: Vec<f32> = samples.iter().filter_map(|sample| sample.0).collect();
            let ping_text = match last.0 {
                Some(ping) => format!("{ping:.0} ms · média {:.0} ms", pings.iter().sum::<f32>() / pings.len().max(1) as f32),
                None => "—".to_owned(),
            };
            let loss_text = last.1.map_or_else(|| "—".to_owned(), |loss| format!("{} %", format!("{loss:.2}").replace('.', ",")));
            ui.add_space(12.0);
            graph(ui, &palette, "Ping", &ping_text, samples, |sample| sample.0, 50.0, 25.0, palette.accent);
            ui.add_space(12.0);
            graph(ui, &palette, "Perda de pacotes", &loss_text, samples, |sample| sample.1, 2.0, 1.0, palette.danger);
            ui.add_space(12.0);
            stats(ui, &palette, &rows);

            let copied = self.debug.copied_at.is_some_and(|at| now - at < 1.5);
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("REGISTRO TÉCNICO").color(palette.muted).size(12.0).family(theme::bold()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    copy = ghost_button(ui, &palette, if copied { "Copiado" } else { "Copiar registro" }).clicked();
                });
            });
            ui.add_space(8.0);
            Frame::new().fill(palette.rail).corner_radius(CornerRadius::same(6)).inner_margin(Margin::same(10)).show(ui, |ui| {
                ui.set_width(inner - 20.0);
                egui::ScrollArea::vertical().max_height(200.0).auto_shrink([false, true]).stick_to_bottom(true).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for line in &self.debug.log {
                        ui.add(egui::Label::new(RichText::new(line.as_str()).color(palette.text).size(12.0).monospace()).wrap());
                    }
                });
            });
        });
        if let Some(key) = select {
            self.debug.selected = Some(key);
        }
        if copy {
            context.copy_text(self.debug_log_text());
            self.debug.copied_at = Some(now);
        }
        if close || response.should_close() {
            self.debug.open = false;
        }
    }
}

/// The × in the corner of a dialog, as `.close-button` drew it.
pub(crate) fn close_button(ui: &mut Ui, palette: &Palette) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::click());
    let ink = if response.hovered() { palette.strong } else { palette.muted };
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), palette.hover);
    }
    crate::icons::CLOSE.image(16.0, ink).paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(16.0)));
    response.on_hover_text("Fechar").on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A tab of a dialog, as `.tab` drew it.
fn tab(ui: &mut Ui, palette: &Palette, label: &str, selected: bool) -> bool {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::new(15.0, theme::semibold()), palette.text);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(galley.size().x + 24.0, 30.0), Sense::click());
    let (fill, ink) = if selected {
        (Some(palette.selected), palette.strong)
    } else if response.hovered() {
        (Some(palette.hover), palette.text)
    } else {
        (None, palette.muted)
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
    }
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, ink);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A small button without a background until the pointer is over it, as `.button.ghost.small`.
pub(crate) fn ghost_button(ui: &mut Ui, palette: &Palette, label: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::new(13.0, theme::semibold()), palette.text);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(galley.size().x + 20.0, 28.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), palette.hover);
    }
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, palette.text);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The numbers of a connection in cards of at least 150 pixels, as `.stats` laid them out.
fn stats(ui: &mut Ui, palette: &Palette, rows: &[(&'static str, String)]) {
    if rows.is_empty() {
        return;
    }
    let width = ui.available_width();
    let columns = (((width + 8.0) / (150.0 + 8.0)).floor() as usize).max(1);
    let card_width = (width - 8.0 * (columns - 1) as f32) / columns as f32;
    let lines = rows.len().div_ceil(columns);
    let card_height = 8.0 + 15.0 + 2.0 + 20.0 + 8.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, lines as f32 * card_height + (lines - 1) as f32 * 8.0), Sense::hover());
    for (index, (name, value)) in rows.iter().enumerate() {
        let card = Rect::from_min_size(
            rect.min + Vec2::new((index % columns) as f32 * (card_width + 8.0), (index / columns) as f32 * (card_height + 8.0)),
            Vec2::new(card_width, card_height),
        );
        ui.painter().rect_filled(card, CornerRadius::same(6), palette.sidebar);
        let painter = ui.painter().with_clip_rect(card);
        painter.text(card.min + Vec2::new(10.0, 8.0), egui::Align2::LEFT_TOP, name.to_uppercase(), FontId::proportional(11.0), palette.muted);
        painter.text(card.min + Vec2::new(10.0, 8.0 + 15.0 + 2.0), egui::Align2::LEFT_TOP, value, FontId::proportional(15.0), palette.strong);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_reports_of_an_engine_become_lines_samples_and_frames_per_second() {
        let mut view = DebugView::default();
        let report = |frames: u64, rtt: f64| json!({ "event": "stats", "role": "sender", "frames": frames, "width": 1920, "height": 1080, "rtt_ms": rtt, "loss": 0.015, "bitrate_bps": 6_250_000.0, "viewers": 2 });
        view.engine_event("outgoing", "Sua transmissão", "resumo", &report(100, 20.0), 10.0);
        view.engine_event("outgoing", "Sua transmissão", "resumo", &report(160, 30.0), 11.0);
        let source = &view.sources[0];
        assert_eq!(source.samples.len(), 2);
        assert_eq!(source.samples[1], (Some(30.0), Some(1.5)));
        let row = |name: &str| source.rows.iter().find(|(known, _)| *known == name).map(|(_, value)| value.clone()).unwrap();
        assert_eq!(row("Quadros por segundo"), "60");
        assert_eq!(row("Resolução"), "1920×1080");
        assert_eq!(row("Perda de pacotes"), "1,50 %");
        assert_eq!(row("Taxa de envio"), "6,25 Mbps");
        assert_eq!(row("Encode"), "—");
        assert_eq!(row("Assistindo"), "2");
        assert_eq!(view.selected.as_deref(), Some("outgoing"));

        // Who watches gets other lines, and an error goes to the log, not to the numbers.
        view.engine_event("peer-1", "Tela de Keep", "", &json!({ "event": "stats", "role": "receiver", "presented": 30, "dropped": 2 }), 12.0);
        assert!(view.sources[1].rows.iter().any(|(name, value)| *name == "Descartados" && value == "2"));
        view.engine_event("peer-1", "Tela de Keep", "", &json!({ "event": "error", "stage": "network", "status": "x", "message": "caiu" }), 13.0);
        assert_eq!(view.log.back().unwrap(), "[Tela de Keep] erro network: x caiu");
        assert_eq!(view.sources[1].samples.len(), 1);

        view.remove("outgoing");
        assert_eq!((view.sources.len(), view.selected.as_deref()), (1, Some("peer-1")));
        for index in 0..200 {
            view.engine_event("peer-1", "Tela de Keep", "", &json!({ "event": "stats", "role": "receiver", "presented": index }), 14.0 + f64::from(index));
        }
        assert_eq!(view.sources[0].samples.len(), MAX_SAMPLES);
    }
}
