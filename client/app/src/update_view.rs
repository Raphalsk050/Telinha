//! The notice of a new version, and the restart that puts it in place.

use std::sync::Arc;

use egui::{Align2, CornerRadius, Frame, Margin, RichText, Stroke, Vec2};
use telinha_core::updater::{UpdateState, UpdateStatus, Updater};

use crate::app::TelinhaApp;
use crate::dialogs::{Tone, button};

/// With the new version downloaded, the app restarts by itself after this long without a call or
/// a share.
const IDLE_BEFORE_RESTART: f64 = 15.0;

#[derive(Default)]
pub struct UpdateView {
    updater: Option<Arc<Updater>>,
    /// "Depois" holds the restart until the app is closed.
    postponed: bool,
    idle_since: Option<f64>,
    /// The notice the person sent away. It comes back when it has something else to say.
    dismissed: Option<(UpdateStatus, Option<String>)>,
    /// A made-up state to draw, for the tests.
    preview: Option<UpdateState>,
}

impl TelinhaApp {
    pub fn set_updater(&mut self, updater: Arc<Updater>) {
        self.update.updater = Some(updater);
    }

    /// Draws the notice as if the updater were in this state. For the tests.
    pub fn show_update_for_test(&mut self, state: Option<UpdateState>) {
        self.update.preview = state;
    }

    fn busy_for_update(&self) -> bool {
        self.room.voice.is_some() || self.manual.is_some() || self.room.streams.outgoing.is_some() || !self.room.streams.incoming.is_empty()
    }

    fn restart_to_update(&mut self, context: &egui::Context) {
        let Some(updater) = &self.update.updater else { return };
        if updater.apply(std::process::id(), true, true) {
            self.hub.log().info("fechando para trocar o executavel pela versao nova");
            context.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    pub(crate) fn tend_update(&mut self, context: &egui::Context) {
        let state = match (&self.update.preview, &self.update.updater) {
            (Some(preview), _) => preview.clone(),
            (None, Some(updater)) => updater.snapshot(),
            (None, None) => return,
        };
        let now = context.input(|input| input.time);
        if self.update.preview.is_none() {
            // The updater works on its own and does not wake the window, so the window looks
            // again by itself: often while a download moves, now and then otherwise.
            let again = if state.status == UpdateStatus::Downloading { 0.5 } else { 5.0 };
            context.request_repaint_after(std::time::Duration::from_secs_f64(again));
            if self.update.postponed || state.status != UpdateStatus::Ready || self.busy_for_update() {
                self.update.idle_since = None;
            } else {
                let since = *self.update.idle_since.get_or_insert(now);
                if now - since >= IDLE_BEFORE_RESTART {
                    self.restart_to_update(context);
                    return;
                }
                context.request_repaint_after(std::time::Duration::from_secs_f64((IDLE_BEFORE_RESTART - (now - since)).max(0.1)));
            }
        }

        let version = state.version.clone().unwrap_or_default();
        let (text, action) = match state.status {
            UpdateStatus::Idle => return,
            UpdateStatus::Downloading => (format!("Baixando o Telinha {version}… {}%", (state.progress * 100.0).floor()), None),
            UpdateStatus::Ready => (format!("O Telinha {version} está pronto. O app reinicia sozinho quando você não estiver em chamada."), Some("Reiniciar agora")),
            UpdateStatus::Blocked => (format!("Faltou a permissão do Windows para trocar o Telinha pelo {version}."), Some("Tentar de novo")),
        };
        if action.is_some() && self.update.dismissed.as_ref() == Some(&(state.status, state.version.clone())) {
            return;
        }
        let palette = self.palette;
        let mut restart = false;
        let mut later = false;
        egui::Area::new(egui::Id::unique("update-notice")).order(egui::Order::Foreground).anchor(Align2::CENTER_TOP, Vec2::new(0.0, 12.0)).show(context, |ui| {
            Frame::new()
                .fill(palette.float)
                .corner_radius(CornerRadius::same(8))
                .stroke(Stroke::new(1.0, palette.line_soft))
                .inner_margin(Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(text).color(palette.strong).size(14.0));
                        if let Some(action) = action {
                            ui.add_space(6.0);
                            restart = button(ui, &palette, action, Tone::Primary).clicked();
                            later = crate::debug_view::ghost_button(ui, &palette, "Depois").clicked();
                        }
                    });
                });
        });
        if later {
            self.update.postponed = true;
            self.update.dismissed = Some((state.status, state.version));
        }
        if restart {
            self.restart_to_update(context);
        }
    }
}
