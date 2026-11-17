//! The pinned messages of the conversation open, in a balloon under the pin of the header, as
//! Discord shows them: who wrote each, when, what it says, and the way to it or to unpin it.

use egui::{Align2, Color32, CornerRadius, FontId, Frame, Margin, Rect, RichText, Sense, Stroke, Vec2};

use crate::app::TelinhaApp;
use crate::icons;
use crate::theme;
use crate::widgets;

const WIDTH: f32 = 420.0;

impl TelinhaApp {
    pub(crate) fn show_pins(&mut self, context: &egui::Context) {
        let open = crate::motion::ease(context, egui::Id::unique("pins-open"), self.pins_open, crate::motion::OPEN);
        // It fades out after it closes, as it faded in.
        if open <= 0.0 {
            return;
        }
        let Some(button) = self.pins_button else {
            self.pins_open = false;
            return;
        };
        let palette = self.palette;
        let pinned = self.pinned_messages();
        let hub = std::sync::Arc::clone(self.hub());
        let mut jump = None;
        let mut unpin = None;
        // On glass the balloon is a pane of glass drawn under what it holds.
        let frame = if palette.glass {
            Frame::new().inner_margin(Margin::same(12))
        } else {
            Frame::new()
                .fill(palette.sidebar)
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::same(12))
                .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(128) })
        };
        let margin = frame.inner_margin;
        // It drops a little into place as it opens.
        let anchor = egui::pos2(button.right() + 4.0, button.bottom() + 10.0 - (1.0 - open) * 6.0);
        let area = egui::Area::new(egui::Id::unique("pins"))
            .order(egui::Order::Foreground)
            .pivot(Align2::RIGHT_TOP)
            .fixed_pos(anchor)
            .interactable(self.pins_open)
            .show(context, |ui| {
                ui.multiply_opacity(open);
                frame.show(ui, |ui| {
                    crate::glass::panel_contents(ui, margin, 14.0, |ui| {
                        ui.set_width(WIDTH);
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            ui.add(icons::PIN.image(18.0, palette.strong));
                            ui.label(RichText::new("Mensagens fixadas").color(palette.strong).size(16.0).family(theme::semibold()));
                            if !pinned.is_empty() {
                                ui.label(RichText::new(pinned.len().to_string()).color(palette.muted).size(14.0));
                            }
                        });
                        ui.add_space(12.0);
                        if pinned.is_empty() {
                            empty(ui, &palette);
                            return;
                        }
                        // A few fit as they are; more scroll, below the title.
                        let mut cards = |ui: &mut egui::Ui| {
                            ui.spacing_mut().item_spacing.y = 8.0;
                            for message in &pinned {
                                match self.pinned_card(ui, &hub, message) {
                                    Some(Choice::Jump) => jump = Some(message.id.clone()),
                                    Some(Choice::Unpin) => unpin = Some(message.id.clone()),
                                    None => {}
                                }
                            }
                        };
                        if pinned.len() <= 3 {
                            cards(ui);
                        } else {
                            egui::ScrollArea::vertical().max_height(440.0).auto_shrink([false, true]).show(ui, cards);
                        }
                    })
                });
            });
        if let Some(id) = jump {
            self.pins_open = false;
            self.jump_to_message(&id);
        }
        if let Some(id) = unpin {
            self.unpin(&id);
        }
        if !self.pins_open {
            return;
        }
        // A click outside closes it, but not the one on its own button, nor one in a menu.
        let on_button = context.input(|input| input.pointer.interact_pos()).is_some_and(|pos| button.contains(pos));
        if area.response.clicked_elsewhere() && !on_button && !egui::Popup::is_any_open(context) && self.dialog.is_none() {
            self.pins_open = false;
        }
        if context.input(|input| input.key_pressed(egui::Key::Escape)) && self.dialog.is_none() {
            self.pins_open = false;
        }
    }

    /// One pinned message: the face and the name of who wrote it, when, and what it says. On
    /// hover the ways to go to it and to unpin it show at its top right.
    fn pinned_card(&mut self, ui: &mut egui::Ui, hub: &telinha_core::hub::Hub, message: &telinha_core::chats::Message) -> Option<Choice> {
        let palette = self.palette;
        let member = if message.mine { Some(hub.member_id()) } else { message.member_id.clone() };
        let (fill, round) = if palette.glass { (Color32::from_white_alpha(10), 10) } else { (palette.main, 6) };
        let card = Frame::new().fill(fill).corner_radius(CornerRadius::same(round)).inner_margin(Margin { left: 12, right: 12, top: 10, bottom: 12 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                widgets::avatar(ui, 32.0, &message.author, self.pictures.avatar(hub, member.as_deref()));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.set_max_width(ui.available_width() - 4.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(RichText::new(&message.author).color(palette.strong).size(14.0).family(theme::semibold()));
                        ui.label(RichText::new(crate::chat_view::format_time(message.sent_at)).color(palette.muted).size(12.0));
                    });
                    if !message.text.is_empty() {
                        let text = if message.text.chars().count() > 300 { format!("{}…", message.text.chars().take(300).collect::<String>()) } else { message.text.clone() };
                        ui.add(egui::Label::new(RichText::new(text).color(palette.text).size(14.0)).wrap());
                    }
                    let carries = if message.image.is_some() {
                        Some("Imagem".to_owned())
                    } else {
                        message.file.as_ref().map(|file| format!("Arquivo: {}", file.name))
                    };
                    if let Some(carries) = carries {
                        ui.label(RichText::new(carries).color(palette.muted).size(13.0).italics());
                    }
                    if message.thread_id().is_some() {
                        ui.label(RichText::new("Resposta em um tópico").color(palette.muted).size(12.0));
                    }
                });
            });
        });
        let rect = card.response.rect;
        let hovered = ui.rect_contains_pointer(rect);
        let lit = crate::motion::hover(ui.ctx(), ui.scope_id().with(("pin-card", &message.id)), hovered);
        if palette.glass {
            ui.painter().rect_stroke(rect, CornerRadius::same(round), Stroke::new(1.0, crate::glass::EDGE.gamma_multiply(0.4 + 0.6 * lit)), egui::StrokeKind::Inside);
        }
        let mut choice = None;
        // Clicking the card goes to the message, as Discord's "Ir".
        let whole = ui.interact(rect, ui.scope_id().with(("pin-go", &message.id)), Sense::click());
        if lit > 0.0 {
            let go_galley = ui.painter().layout_no_wrap("Ir".to_owned(), FontId::new(12.0, theme::semibold()), palette.strong);
            let go = Rect::from_min_size(egui::pos2(rect.right() - 8.0 - 26.0 - 6.0 - (go_galley.size().x + 20.0), rect.top() + 8.0), Vec2::new(go_galley.size().x + 20.0, 24.0));
            let close = Rect::from_min_size(egui::pos2(rect.right() - 8.0 - 26.0, rect.top() + 7.0), Vec2::splat(26.0));
            let go_response = ui.interact(go, ui.scope_id().with(("pin-go-button", &message.id)), Sense::click());
            let go_fill = if go_response.hovered() { palette.selected } else { palette.raised };
            ui.painter().rect_filled(go, CornerRadius::same(if palette.glass { 8 } else { 4 }), go_fill.gamma_multiply(lit));
            ui.painter().galley_with_override_text_color(go.center() - go_galley.size() / 2.0, go_galley, palette.strong.gamma_multiply(lit));
            let close_response = ui.interact(close, ui.scope_id().with(("pin-unpin", &message.id)), Sense::click());
            let close_lit = crate::motion::hover(ui.ctx(), close_response.id, close_response.hovered());
            ui.painter().circle_filled(close.center(), 13.0, crate::motion::mix(palette.raised, palette.danger, close_lit).gamma_multiply(lit));
            icons::CLOSE.image(14.0, crate::motion::mix(palette.soft, Color32::WHITE, close_lit).gamma_multiply(lit)).paint_at(ui, Rect::from_center_size(close.center(), Vec2::splat(14.0)));
            if go_response.on_hover_text("Ir até a mensagem").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                choice = Some(Choice::Jump);
            }
            if close_response.on_hover_text("Desafixar").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                choice = Some(Choice::Unpin);
            }
        }
        if choice.is_none() && whole.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            choice = Some(Choice::Jump);
        }
        choice
    }
}

enum Choice {
    Jump,
    Unpin,
}

/// The balloon of a conversation without pinned messages: the pin in a circle and what to do.
fn empty(ui: &mut egui::Ui, palette: &crate::theme::Palette) {
    ui.vertical_centered(|ui| {
        ui.add_space(16.0);
        let (circle, _) = ui.allocate_exact_size(Vec2::splat(72.0), Sense::hover());
        ui.painter().circle_filled(circle.center(), 36.0, if palette.glass { Color32::from_white_alpha(15) } else { palette.raised });
        icons::PIN.image(32.0, palette.soft).paint_at(ui, Rect::from_center_size(circle.center(), Vec2::splat(32.0)));
        ui.add_space(14.0);
        ui.label(RichText::new("Nenhuma mensagem fixada ainda").color(palette.strong).size(15.0).family(theme::semibold()));
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new("Passe o mouse sobre uma mensagem e clique no alfinete para guardá-la aqui, à mão de todo mundo da conversa.").color(palette.muted).size(13.0)).wrap());
        ui.add_space(16.0);
    });
}
