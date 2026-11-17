//! The window: what is on screen and what happens when the person clicks.

use std::sync::Arc;

use egui::{Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Panel, Rect, RichText, Sense, Ui, Vec2};
use telinha_core::contacts::Contact;
use telinha_core::hub::{CallKind, Conversation, Hub, HubEvent};
use telinha_core::servers::{ChannelKind, Server};
use telinha_core::signaling::Status;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::alerts::{Alerts, paint_badge};
use crate::appearance::{Appearance, Gradient, Regions};
use crate::chat_view::{ChatAction, ChatView, Chrome};
use crate::crop_editor::{CropAnswer, CropEditor};
use crate::home_view::ManualSession;
use crate::name_style::{CardRequest, Styles, paint_name, paint_row_name};
use crate::lightbox::Lightbox;
use crate::dialogs::{Answer, Danger, Dialog, InviteContact, Purpose};
use crate::icons;
use crate::pictures::Pictures;
use crate::prefs::Prefs;
use crate::room_view::RoomState;
use crate::settings::{Settings, SettingsAction, SettingsView};
use crate::theme::{self, Palette};
use crate::widgets;

const RAIL_WIDTH: f32 = 72.0;
const SIDEBAR_WIDTH: f32 = 240.0;
const MEMBERS_WIDTH: f32 = 240.0;
/// The room around and between the floating panels of the glass theme.
const GAP: f32 = 10.0;

/// Where the person is in the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nav {
    Home,
    Direct(String),
    Server { server_id: String, channel_id: Option<String> },
}

pub struct TelinhaApp {
    pub(crate) hub: Arc<Hub>,
    events: std::sync::mpsc::Receiver<HubEvent>,
    pub(crate) runtime: tokio::runtime::Handle,
    pub(crate) context: egui::Context,
    pub(crate) palette: Palette,
    /// The colors of the rail and of the sidebar, which may stay dark in the light theme.
    pub(crate) side: Palette,
    /// The fill of each region of the window: plain, or a veil over the gradient of the theme.
    regions: Regions,
    backdrop: Option<Gradient>,
    system_light: bool,
    pub(crate) nav: Nav,
    pub(crate) servers: Vec<Server>,
    pub(crate) contacts: Vec<(Contact, bool)>,
    status: Status,
    pub(crate) prefs: Prefs,
    settings: Option<Settings>,
    avatar_picker: Option<crate::avatar_picker::AvatarPicker>,
    /// The microphone heard by the bar of the settings, while they are open.
    mic_test: Option<telinha_voice::audio::MicTest>,
    /// The camera tested in the settings, when the call does not have it on already.
    pub(crate) camera_test: Option<telinha_voice::camera::Camera>,
    /// Where the frames of that test go: only to the settings, never into a call.
    test_feed: Option<telinha_voice::video::CameraFeed>,
    previewing: bool,
    camera_status: String,
    /// The theme changed, and the next draw has to give the new colors to egui itself.
    restyle: bool,
    pub(crate) alerts: Alerts,
    /// The connection with codes that is open on the first screen, when there is one.
    pub(crate) manual: Option<ManualSession>,
    pub(crate) update: crate::update_view::UpdateView,
    pub(crate) styles: Styles,
    crop: Option<CropEditor>,
    pub(crate) soundboard: crate::soundboard_view::SoundboardView,
    pub(crate) debug: crate::debug_view::DebugView,
    /// The profile card that is open, and when it opened in the clock of the window.
    pub(crate) card: Option<CardRequest>,
    pub(crate) card_opened: f64,
    chat: ChatView,
    pub(crate) pictures: Pictures,
    pub(crate) dialog: Option<Dialog>,
    pub(crate) room: RoomState,
    lightbox: Option<Lightbox>,
    /// A short notice at the bottom of the window, and when it goes away.
    toast: Option<(String, f64)>,
    /// The glass drawn without its blur, for the test that checks the blur.
    sharp_glass: bool,
}

impl TelinhaApp {
    pub fn new(
        context: &egui::Context,
        hub: Arc<Hub>,
        events: UnboundedReceiver<HubEvent>,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        egui_extras::install_image_loaders(context);
        theme::install_fonts(context);
        let prefs = Prefs::load(hub.data_dir());
        let system_light = context.system_theme() == Some(egui::Theme::Light);
        let (palette, regions) = prefs.appearance.palette(system_light);
        let side = prefs.appearance.side_palette(system_light).unwrap_or(palette);
        let backdrop = prefs.appearance.gradient();
        theme::apply(context, &palette);
        context.set_zoom_factor(prefs.zoom_factor());
        let servers = hub.servers();
        let nav = match servers.first() {
            Some(server) => Nav::Server { server_id: server.id.clone(), channel_id: first_text_channel(server) },
            None => Nav::Home,
        };
        // Whatever arrives wakes the window, which otherwise only draws when the person moves.
        let (forward, forwarded) = std::sync::mpsc::channel();
        let waker = context.clone();
        let mut events = events;
        runtime.spawn(async move {
            while let Some(event) = events.recv().await {
                if forward.send(event).is_err() {
                    return;
                }
                waker.request_repaint();
            }
        });
        let styles = Styles::load(hub.data_dir());
        let mut app = Self {
            events: forwarded,
            context: context.clone(),
            room: RoomState::new(&hub),
            lightbox: None,
            contacts: hub.contacts(),
            status: hub.signaling_status(),
            pictures: Pictures::new(&hub),
            servers,
            hub,
            runtime,
            palette,
            side,
            regions,
            backdrop,
            system_light,
            nav: Nav::Home,
            prefs,
            settings: None,
            avatar_picker: None,
            mic_test: None,
            camera_test: None,
            test_feed: None,
            previewing: false,
            camera_status: String::new(),
            // The window that draws is not always the one this was made with, as in the tests.
            restyle: true,
            alerts: Alerts::default(),
            manual: None,
            update: crate::update_view::UpdateView::default(),
            styles,
            crop: None,
            soundboard: crate::soundboard_view::SoundboardView::default(),
            debug: crate::debug_view::DebugView::default(),
            card: None,
            card_opened: 0.0,
            chat: ChatView::default(),
            dialog: None,
            toast: None,
            sharp_glass: false,
        };
        app.use_chosen_devices();
        if app.prefs.app_icon != crate::app_icons::DEFAULT {
            crate::app_icons::apply(context, &app.prefs.app_icon);
        }
        app.remember_own_style();
        app.navigate(nav);
        app
    }

    fn remember_own_style(&mut self) {
        let style = self.hub.profile_style();
        self.styles.remember(&self.hub.member_id(), Some(&style));
    }

    /// The fill of the strip of the person and of the call at the foot of the sidebar.
    pub(crate) fn foot_fill(&self) -> Color32 {
        // A sidebar kept dark in the light theme keeps its own dark strip.
        if self.side == self.palette { self.regions.foot } else { self.side.panel }
    }

    pub fn hub(&self) -> &Arc<Hub> {
        &self.hub
    }

    pub fn navigate(&mut self, nav: Nav) {
        self.nav = nav;
        let conversation = self.conversation();
        if let Some(conversation) = &conversation {
            self.mark_read(conversation);
        }
        self.chat.open(&self.hub, conversation);
    }

    fn drain_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                HubEvent::Servers => {
                    self.servers = self.hub.servers();
                    // The channel that was open may be gone, or the server itself.
                    let nav = match &self.nav {
                        Nav::Server { server_id, channel_id } => match self.servers.iter().find(|server| &server.id == server_id) {
                            Some(server) if channel_id.as_ref().is_some_and(|id| server.channels.iter().any(|channel| &channel.id == id)) => None,
                            Some(server) => Some(Nav::Server { server_id: server.id.clone(), channel_id: first_text_channel(server) }),
                            None => Some(Nav::Home),
                        },
                        _ => None,
                    };
                    if let Some(nav) = nav {
                        self.navigate(nav);
                    }
                }
                HubEvent::Contacts => self.contacts = self.hub.contacts(),
                HubEvent::Signaling(status) => self.status = status,
                HubEvent::ChatMessage { conversation, message } => {
                    self.count_unread(&conversation);
                    if !message.mine {
                        self.message_tone();
                    }
                    self.chat.received(&conversation, message);
                }
                HubEvent::Notify { title, body, member_id, open } => self.show_notice(&title, &body, member_id.as_deref(), open),
                HubEvent::IncomingCall { contact_id, name, kind } => self.incoming_call(contact_id, name, kind == CallKind::Live),
                HubEvent::FileStatus { file_id, status, .. } => self.chat.file_status(&file_id, status),
                HubEvent::FileProgress { file_id, done, total, up, .. } => self.chat.file_progress(&file_id, done, total, up),
                HubEvent::UploadProgress { file_id, done, total, .. } => self.chat.file_progress(&file_id, done, total, true),
                HubEvent::Manual(event) => self.manual_event(&event),
                HubEvent::ManualEnded { code, .. } => self.manual_ended(code),
                HubEvent::ContactSaved { contact_id, created, .. } => self.contact_saved(contact_id, created),
                HubEvent::ChatUpdate { conversation, message } => self.chat.updated(&conversation, message),
                HubEvent::ChatRemove { conversation, message_id } => self.chat.removed(&conversation, &message_id),
                HubEvent::Picture { member_id, banner } => self.pictures.forget(&member_id, banner),
                HubEvent::Profile => {
                    self.pictures.forget_own();
                    self.remember_own_style();
                }
                other => {
                    let room_changed = matches!(other, HubEvent::Voice | HubEvent::Peers { .. } | HubEvent::Streams);
                    if let HubEvent::Peers { space_id } = &other {
                        for peer in self.hub.peers(space_id) {
                            if let Some(member_id) = &peer.member_id {
                                self.styles.remember(member_id, peer.style.as_ref());
                            }
                        }
                    }
                    self.room_event(other);
                    if room_changed {
                        self.after_room_change();
                    }
                }
            }
        }
    }

    /// What the chat shows around the messages of the open conversation.
    fn chrome(&self) -> Option<Chrome> {
        let connected = self.status.connected;
        let offline_hint = "Sem conexão com o servidor de contatos. As mensagens não vão chegar agora.";
        let look = &self.prefs.appearance;
        let (font, gap, compact) = (look.chat_font, look.message_gap, look.messages == crate::appearance::MessageStyle::Compact);
        match &self.nav {
            Nav::Home => None,
            Nav::Direct(id) => {
                let (contact, online) = self.contacts.iter().find(|(contact, _)| &contact.id == id)?;
                let hint = if !connected {
                    offline_hint.to_owned()
                } else if !online {
                    format!("{} está offline. A mensagem só chega com o Telinha aberto do outro lado.", contact.name)
                } else {
                    "Enter envia, Shift+Enter pula linha.".to_owned()
                };
                Some(Chrome {
                    title: contact.name.clone(),
                    intro_icon: None,
                    intro_title: contact.name.clone(),
                    intro_text: "Este é o começo da conversa. As mensagens ficam guardadas só neste computador e só chegam quando a outra pessoa está com o Telinha aberto.".to_owned(),
                    placeholder: format!("Conversar com @{}", contact.name),
                    hint,
                    font,
                    gap,
                    compact,
                })
            }
            Nav::Server { server_id, channel_id } => {
                let server = self.servers.iter().find(|server| &server.id == server_id)?;
                let channel = server.channels.iter().find(|channel| Some(&channel.id) == channel_id.as_ref())?;
                let text = channel.kind == ChannelKind::Text;
                Some(Chrome {
                    title: channel.name.clone(),
                    intro_icon: Some(if text { icons::HASH } else { icons::SPEAKER }),
                    intro_title: if text { format!("Bem-vindo a #{}", channel.name) } else { format!("Chat de {}", channel.name) },
                    intro_text: if text {
                        format!("Este é o começo do canal #{} em {}. Cada computador guarda o próprio histórico, e quem está offline não recebe.", channel.name, server.name)
                    } else {
                        "Converse aqui com quem está na sala. As mensagens ficam guardadas só neste computador.".to_owned()
                    },
                    placeholder: if text { format!("Conversar em #{}", channel.name) } else { format!("Conversar em {}", channel.name) },
                    hint: if connected { String::new() } else { offline_hint.to_owned() },
                    font,
                    gap,
                    compact,
                })
            }
        }
    }

    fn current_server(&self) -> Option<&Server> {
        match &self.nav {
            Nav::Server { server_id, .. } => self.servers.iter().find(|server| &server.id == server_id),
            _ => None,
        }
    }

    pub fn conversation(&self) -> Option<Conversation> {
        match &self.nav {
            Nav::Home => None,
            Nav::Direct(contact_id) => Some(Conversation::direct(contact_id)),
            Nav::Server { server_id, channel_id } => Some(Conversation::channel(server_id, channel_id.as_deref()?)),
        }
    }

    /// Draws the whole window. Called by eframe on every frame, and by the tests.
    pub fn ui(&mut self, ui: &mut Ui) {
        let _runtime = self.runtime.enter();
        self.drain_events();
        self.drain_tasks();
        self.tend_alerts(ui);
        let palette = self.palette;
        if std::mem::take(&mut self.restyle) {
            theme::apply(ui.ctx(), &palette);
        }
        // Ctrl with + and - changes the size too, and that choice is kept like the slider's.
        let zoom = ui.ctx().zoom_factor();
        if (zoom - self.prefs.zoom_factor()).abs() > 0.001 {
            self.prefs.appearance.zoom = (zoom * 100.0).clamp(50.0, 200.0);
            self.prefs.save();
        }
        // A theme that follows Windows changes when Windows does.
        let system_light = ui.ctx().system_theme() == Some(egui::Theme::Light);
        if system_light != self.system_light {
            self.system_light = system_light;
            self.apply_appearance();
        }
        let regions = self.regions;
        let glass = self.prefs.appearance.glass_look();
        crate::glass::set_look(ui.ctx(), glass);
        crate::glass::set_sharp(ui.ctx(), self.sharp_glass);
        let gap = if glass.is_some() { GAP } else { 0.0 };
        if let Some(look) = &glass {
            crate::glass::wallpaper(ui.painter(), ui.ctx().content_rect(), look);
        } else if let Some(backdrop) = &self.backdrop {
            let time = ui.input(|input| input.time);
            backdrop.paint(ui.painter(), ui.ctx().content_rect(), time);
            if backdrop.motion {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
            }
        }

        // In the glass theme the panels float, 10 points apart and from the edges, as
        // `#app { padding: 10px 10px 10px 0; gap: 10px }`.
        Panel::left("rail")
            .exact_size(RAIL_WIDTH + gap)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(regions.rail))
            .show(ui, |ui| self.rail(ui));
        Panel::left("sidebar")
            .exact_size(SIDEBAR_WIDTH + gap)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(if glass.is_some() { Color32::TRANSPARENT } else { regions.side }))
            .show(ui, |ui| self.in_pane(ui, regions.side, Self::sidebar));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(if glass.is_some() { Color32::TRANSPARENT } else { regions.main }))
            .show(ui, |ui| self.in_pane(ui, regions.main, Self::main));
        if let Some(lightbox) = &mut self.lightbox {
            if !lightbox.show(ui.ctx(), &palette) {
                self.lightbox = None;
            }
        }
        self.show_viewer(ui.ctx());
        self.show_settings(ui.ctx());
        self.show_avatar_picker(ui.ctx());
        self.show_crop(ui.ctx());
        self.show_soundboard(ui.ctx());
        self.show_debug(ui.ctx());
        self.show_voice_popover(ui.ctx());
        self.show_dialog(ui.ctx());
        self.show_picker(ui.ctx());
        self.show_stream_settings(ui.ctx());
        self.show_incoming(ui.ctx());
        self.show_card(ui.ctx());
        self.tend_update(ui.ctx());
        self.show_toast(ui);
        // Everything is painted: what is behind each glass is known now.
        crate::glass::finish(ui.ctx());
    }

    /// The corners of the panels: rounded in the glass theme, where they float.
    pub(crate) fn corner(&self) -> u8 {
        if self.palette.glass { 16 } else { 0 }
    }

    /// Draws `body` in the whole panel, or, in the glass theme, in a floating glass inside it
    /// with the gap on the top, the right and the bottom.
    fn in_pane(&mut self, ui: &mut Ui, tint: Color32, body: fn(&mut Self, &mut Ui)) {
        if !self.palette.glass {
            body(self, ui);
            return;
        }
        let full = ui.max_rect();
        let rect = Rect::from_min_max(egui::pos2(full.left(), full.top() + GAP), egui::pos2(full.right() - GAP, full.bottom() - GAP));
        crate::glass::pane(ui.painter(), rect, &crate::glass::Pane::panel(tint));
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            body(self, ui);
        });
    }

    fn rail(&mut self, ui: &mut Ui) {
        let palette = self.side;
        ui.add_space(12.0 + if palette.glass { GAP } else { 0.0 });
        ui.allocate_ui_with_layout(Vec2::new(RAIL_WIDTH, ui.available_height()), Layout::top_down(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.y = self.prefs.appearance.density.rail_gap();
            let home = self.nav == Nav::Home || matches!(self.nav, Nav::Direct(_));
            let waiting = self.unread_in_contacts() > 0;
            if rail_button(ui, &palette, home, waiting, RailFace::Icon(icons::HOME), "Mensagens diretas").clicked() {
                self.navigate(Nav::Home);
            }
            let (line, _) = ui.allocate_exact_size(Vec2::new(32.0, 2.0), Sense::hover());
            ui.painter().rect_filled(line, CornerRadius::same(1), palette.line_soft);

            let mut go = None;
            for server in &self.servers {
                let selected = self.current_server().is_some_and(|current| current.id == server.id);
                let face = RailFace::Text(server_initials(&server.name));
                let waiting = self.unread_in_server(&server.id) > 0;
                let response = rail_button(ui, &palette, selected, waiting, face, &server.name);
                // A green dot on the corner of the server whose room the person is in.
                if self.room.voice.as_ref().is_some_and(|voice| voice.space_id == server.id) {
                    let face = Rect::from_center_size(response.rect.center(), Vec2::splat(48.0));
                    let center = face.right_bottom() - Vec2::splat(5.0);
                    ui.painter().circle_filled(center, 7.0, palette.rail);
                    ui.painter().circle_filled(center, 4.0, palette.green);
                }
                if response.clicked() {
                    go = Some(Nav::Server { server_id: server.id.clone(), channel_id: first_text_channel(server) });
                }
            }
            if let Some(nav) = go {
                self.navigate(nav);
            }
            if rail_button(ui, &palette, false, false, RailFace::Add, "Criar ou entrar em um servidor").clicked() {
                self.dialog = Some(Dialog::server(&self.hub.profile_name()));
            }
        });
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        let palette = self.side;
        let (header, header_response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), self.prefs.appearance.density.header_height()), Sense::click());
        match self.current_server().cloned() {
            Some(server) => {
                if header_response.hovered() {
                    let round = self.corner();
                    ui.painter().rect_filled(header, CornerRadius { nw: round, ne: round, sw: 0, se: 0 }, palette.hover);
                }
                self.server_menu(&header_response, &server);
                widgets::paint_text_clipped(ui, header.left_center() + Vec2::new(16.0, 0.0), header.width() - 56.0, &server.name, FontId::new(15.0, theme::semibold()), palette.strong);
                let chevron = Rect::from_center_size(header.right_center() - Vec2::new(24.0, 0.0), Vec2::splat(16.0));
                icons::CHEVRON_DOWN.image(16.0, palette.soft).paint_at(ui, chevron);
            }
            // Out of a server, the top of the sidebar is the way to the first screen, where a
            // connection with codes starts.
            None => {
                let field = Rect::from_center_size(header.center(), Vec2::new(header.width() - 20.0, 28.0));
                ui.painter().rect_filled(field, CornerRadius::same(4), palette.rail);
                ui.painter().text(field.left_center() + Vec2::new(8.0, 0.0), Align2::LEFT_CENTER, "Conectar com código", FontId::proportional(14.0), palette.muted);
                if header_response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.navigate(Nav::Home);
                }
            }
        }
        ui.painter().hline(header.x_range(), header.bottom(), egui::Stroke::new(1.0, palette.line));

        let bottom = if self.call_panel_shown() { 52.0 + crate::room_view::CALL_PANEL_HEIGHT } else { 52.0 } + FOOT_MARGIN;
        let list_height = (ui.available_height() - bottom).max(0.0);
        ui.allocate_ui(Vec2::new(ui.available_width(), list_height), |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                Frame::new().inner_margin(Margin::symmetric(8, 8)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    match self.current_server().cloned() {
                        Some(server) => self.channel_list(ui, &server),
                        None => self.contact_list(ui),
                    }
                });
            });
        });
        // The call and the person share one card that floats over the foot of the sidebar, as
        // in Discord, with room around it.
        let area = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), ui.available_height()));
        let card = Rect::from_min_max(area.min + Vec2::new(FOOT_MARGIN, 0.0), area.max - Vec2::splat(FOOT_MARGIN));
        let palette = self.side;
        if palette.glass {
            ui.painter().add(egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(60) }.as_shape(card, CornerRadius::same(10)));
            ui.painter().rect_filled(card, CornerRadius::same(10), Color32::from_white_alpha(13));
            ui.painter().rect_stroke(card, CornerRadius::same(10), egui::Stroke::new(1.0, palette.line_soft), egui::StrokeKind::Inside);
        } else {
            ui.painter().rect_filled(card, CornerRadius::same(8), self.foot_fill());
        }
        ui.allocate_space(area.size());
        let mut inside = ui.new_child(egui::UiBuilder::new().max_rect(card).layout(Layout::top_down(Align::Min)));
        inside.spacing_mut().item_spacing = Vec2::ZERO;
        let with_call = self.call_panel_shown();
        self.voice_panel(&mut inside);
        if with_call {
            let y = inside.cursor().min.y;
            inside.painter().hline((card.left() + 10.0)..=(card.right() - 10.0), y, egui::Stroke::new(1.0, palette.line_soft));
        }
        self.user_panel(&mut inside);
    }

    fn channel_list(&mut self, ui: &mut Ui, server: &Server) {
        let palette = self.side;
        for (kind, heading) in [(ChannelKind::Text, "CANAIS DE TEXTO"), (ChannelKind::Voice, "CANAIS DE VOZ")] {
            if section_heading(ui, &palette, heading, true) {
                self.dialog = Some(Dialog::channel(&server.id, None, kind));
            }
            for channel in server.channels.iter().filter(|channel| channel.kind == kind) {
                let selected = matches!(&self.nav, Nav::Server { channel_id: Some(id), .. } if id == &channel.id);
                let (rect, response) = widgets::channel_row(ui, 22.0 + 2.0 * self.prefs.appearance.density.row_pad(), selected, &palette);
                let icon = if kind == ChannelKind::Text { icons::HASH } else { icons::SPEAKER };
                let icon_rect = Rect::from_center_size(rect.left_center() + Vec2::new(18.0, 0.0), Vec2::splat(20.0));
                icon.image(20.0, palette.muted).paint_at(ui, icon_rect);
                let unread = self.unread(&Conversation::channel(&server.id, &channel.id));
                let lit = selected || unread > 0;
                // The room the person is in reads green, as `.channel-button.joined`.
                let joined = self.room.voice.as_ref().is_some_and(|voice| voice.space_id == server.id && voice.room_id == channel.id);
                let color = if joined { palette.green } else if lit { palette.strong } else { palette.soft };
                widgets::paint_text_clipped(
                    ui,
                    rect.left_center() + Vec2::new(38.0, 0.0),
                    rect.width() - 46.0 - if unread > 0 { 30.0 } else { 0.0 },
                    &channel.name,
                    FontId::new(15.0, if lit { theme::semibold() } else { egui::FontFamily::Proportional }),
                    color,
                );
                // The gear shows at the right end while the pointer is on the row: the time of
                // the call and the count step aside for it.
                let aside = if ui.rect_contains_pointer(rect) { 8.0 + 28.0 } else { 8.0 };
                if let Some(timer) = self.room_timer(&server.id, &channel.id) {
                    ui.painter().text(rect.right_center() - Vec2::new(aside, 0.0), Align2::RIGHT_CENTER, timer, FontId::proportional(12.0), palette.green);
                } else if unread > 0 {
                    paint_badge(ui, rect.right_center() - Vec2::new(aside, 0.0), unread, &palette);
                }
                // The gear that edits the channel shows over the row while the pointer is on it.
                let mut gear_clicked = false;
                if ui.rect_contains_pointer(rect) {
                    let gear = Rect::from_min_size(egui::pos2(rect.right() - 6.0 - 22.0, rect.center().y - 11.0), Vec2::splat(22.0));
                    let gear_response = ui.interact(gear, ui.scope_id().with(("channel-gear", &channel.id)), Sense::click());
                    let ink = if gear_response.hovered() { palette.strong } else { palette.muted };
                    icons::GEAR.image(16.0, ink).paint_at(ui, Rect::from_center_size(gear.center(), Vec2::splat(16.0)));
                    gear_clicked = gear_response.on_hover_text("Editar canal").on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
                }
                if gear_clicked {
                    self.dialog = Some(Dialog::channel(&server.id, Some((&channel.id, &channel.name, channel.kind)), kind));
                } else if response.clicked() {
                    self.navigate(Nav::Server { server_id: server.id.clone(), channel_id: Some(channel.id.clone()) });
                    // A click on a voice channel enters it, as it did in the old app.
                    if kind == ChannelKind::Voice && !self.room.voice.as_ref().is_some_and(|voice| voice.space_id == server.id && voice.room_id == channel.id) {
                        self.join_room(&server.id, &channel.id);
                    }
                }
                if kind == ChannelKind::Voice {
                    self.voice_rows(ui, server, channel);
                }
            }
            ui.add_space(10.0);
        }
    }

    fn contact_list(&mut self, ui: &mut Ui) {
        let palette = self.side;
        let _ = section_heading(ui, &palette, "MENSAGENS DIRETAS", false);
        // The dot at the end of the heading tells whether the app reaches the message servers.
        let heading = ui.min_rect();
        let light = egui::pos2(heading.right() - 12.0, heading.bottom() - 14.0);
        ui.painter().circle_filled(light, 4.0, if self.status.connected { palette.green } else { palette.muted });
        if self.contacts.is_empty() {
            Frame::new().inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                ui.label(RichText::new("Conecte com alguém usando um código uma vez e a pessoa aparece aqui para as próximas vezes.").color(palette.muted).size(13.0));
            });
        }
        let mut go = None;
        let contacts = self.contacts.clone();
        for (contact, online) in &contacts {
            let selected = matches!(&self.nav, Nav::Direct(id) if id == &contact.id);
            let (rect, response) = widgets::row(ui, 32.0 + 2.0 * self.prefs.appearance.density.row_pad(), selected, &palette);
            let avatar = Rect::from_center_size(rect.left_center() + Vec2::new(24.0, 0.0), Vec2::splat(32.0));
            let member = self.hub.member_of_contact(&contact.id);
            let picture = self.pictures.avatar(&self.hub, member.as_deref());
            widgets::paint_avatar(ui, avatar, &contact.name, picture);
            let backdrop = if selected { palette.selected } else { palette.sidebar };
            widgets::status_dot(ui, avatar, *online, &palette, backdrop);
            let unread = self.unread(&Conversation::direct(&contact.id));
            let room = rect.width() - 58.0 - if unread > 0 { 30.0 } else { 0.0 };
            paint_name(
                ui,
                rect.left_center() + Vec2::new(50.0, -8.0),
                room,
                &contact.name,
                FontId::new(15.0, if unread > 0 { theme::semibold() } else { egui::FontFamily::Proportional }),
                if selected || unread > 0 { palette.strong } else { palette.soft },
                self.styles.of(member.as_deref()),
            );
            // Under the name, what the person is doing, as `contactStatus` said it.
            let peers = self.hub.peers(&contact.id);
            let calling: Vec<_> = peers.iter().filter_map(|peer| peer.voice.as_ref()).filter(|voice| voice.room_id == telinha_core::hub::DM_ROOM).collect();
            let status = if calling.iter().any(|voice| voice.live) {
                "transmitindo"
            } else if !calling.is_empty() {
                "em chamada"
            } else if !peers.is_empty() || *online {
                "online"
            } else {
                "offline"
            };
            widgets::paint_text_clipped(ui, rect.left_center() + Vec2::new(50.0, 9.0), room, status, FontId::proportional(12.0), palette.muted);
            if unread > 0 {
                paint_badge(ui, rect.right_center() - Vec2::new(8.0, 0.0), unread, &palette);
            }
            if response.clicked() {
                go = Some(Nav::Direct(contact.id.clone()));
            }
            // The same menu as for anyone else, as `personFromContact` made it.
            let peers = self.hub.peers(&contact.id);
            let in_call = |peer: &&telinha_core::signaling::Peer| peer.voice.as_ref().is_some_and(|voice| voice.room_id == telinha_core::hub::DM_ROOM);
            let peer = peers.iter().find(in_call).or(peers.first());
            let in_room = peer.is_some_and(|peer| in_call(&peer)) && self.room.voice.as_ref().is_some_and(|voice| voice.space_id == contact.id && voice.room_id == telinha_core::hub::DM_ROOM);
            let person = crate::room_view::Person {
                id: peer.map(|peer| peer.id.clone()).unwrap_or_default(),
                name: contact.name.clone(),
                member_id: peer.and_then(|peer| peer.member_id.clone()),
                mic: false,
                deaf: false,
                live: false,
                camera: false,
                me: false,
            };
            self.person_menu(&response, &person, &contact.id, in_room);
        }
        if let Some(nav) = go {
            self.navigate(nav);
        }
    }

    fn user_panel(&mut self, ui: &mut Ui) {
        let palette = self.side;
        // The card under it is drawn by the sidebar.
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ui.available_height()), Sense::hover());
        let foot = self.foot_fill();
        let name = self.hub.profile_name();
        // The avatar and the name are one button, as `.user-button`: a click opens the settings,
        // a right click the menu of the avatar.
        let area = Rect::from_min_max(egui::pos2(rect.left() + 6.0, rect.center().y - 20.0), egui::pos2(rect.right() - 8.0 - 3.0 * 34.0, rect.center().y + 20.0));
        let user_button = ui.interact(area, ui.scope_id().with("user-button"), Sense::click());
        let lit = crate::motion::hover(ui.ctx(), user_button.id, user_button.hovered());
        if lit > 0.0 {
            ui.painter().rect_filled(area, CornerRadius::same(8), palette.hover.gamma_multiply(lit));
        }
        let avatar = Rect::from_center_size(rect.left_center() + Vec2::new(28.0, 0.0), Vec2::splat(32.0));
        let own = self.hub.member_id();
        let picture = self.pictures.avatar(&self.hub, Some(&own));
        let has_avatar = picture.is_some();
        widgets::paint_avatar(ui, avatar, &name, picture);
        widgets::status_dot(ui, avatar, true, &palette, if foot.a() == 255 { foot } else { palette.panel });
        let text_left = rect.left_center() + Vec2::new(52.0, 0.0);
        paint_name(ui, text_left - Vec2::new(0.0, 8.0), rect.width() - 52.0 - 104.0, &name, FontId::new(14.0, theme::semibold()), palette.strong, self.styles.of(Some(&own)));
        let (mut change, mut remove) = (false, false);
        context_menu(&user_button, &self.palette, |ui| {
            menu_header(ui, &self.palette, &name);
            change = menu_item(ui, &self.palette, "Mudar avatar", false);
            remove = has_avatar && menu_item(ui, &self.palette, "Remover avatar", true);
        });
        if user_button.on_hover_text("Seu nome e dispositivos").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.open_settings();
        }
        if change {
            if let Some(bytes) = crate::settings::pick_picture_file() {
                match bytes {
                    Ok(bytes) => self.open_crop(&bytes, false),
                    Err(text) => self.notice(text),
                }
            }
        }
        if remove {
            self.hub.remove_picture(false);
            self.pictures.forget_own();
        }
        let state = if self.status.connected { "online" } else { "sem conexão" };
        widgets::paint_text_clipped(ui, text_left + Vec2::new(0.0, 9.0), 90.0, state, FontId::proportional(12.0), palette.muted);

        let mut buttons = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(8.0, 10.0))).layout(Layout::right_to_left(Align::Center)));
        buttons.spacing_mut().item_spacing.x = 2.0;
        if widgets::icon_button(&mut buttons, icons::GEAR, 32.0, palette.soft, palette.selected, "Configurações").clicked() {
            self.open_settings();
        }
        let voice = self.room.voice.clone();
        let deaf = voice.as_ref().is_some_and(|voice| voice.deaf);
        // Deafened means the microphone is silent too, as in Discord.
        let mic = voice.as_ref().is_none_or(|voice| voice.mic && !voice.deaf);
        let (deaf_icon, deaf_ink, deaf_tip) = if deaf { (icons::DEAF, palette.danger_text, "Voltar a ouvir") } else { (icons::HEADPHONES, palette.soft, "Ensurdecer") };
        if widgets::icon_button(&mut buttons, deaf_icon, 32.0, deaf_ink, palette.selected, deaf_tip).clicked() && voice.is_some() {
            self.hub.update_voice(None, None, Some(!deaf));
        }
        let (mic_icon, mic_ink, mic_tip) = if mic { (icons::MIC, palette.soft, "Silenciar microfone") } else { (icons::MIC_OFF, palette.danger_text, "Ativar microfone") };
        if widgets::icon_button(&mut buttons, mic_icon, 32.0, mic_ink, palette.selected, mic_tip).clicked() && voice.is_some() {
            // Turning the microphone on while deafened also brings the sound back.
            self.hub.update_voice(Some(!mic), None, (!mic && deaf).then_some(false));
        }
    }

    fn members(&mut self, ui: &mut Ui) {
        let palette = self.palette;
        let Some(server) = self.current_server().cloned() else {
            return;
        };
        let peers = self.hub.peers(&server.id);
        let my_id = self.hub.member_id();
        let is_online = |member_id: &str| member_id == my_id || peers.iter().any(|peer| peer.member_id.as_deref() == Some(member_id));
        let mut online: Vec<_> = server.members.iter().filter(|member| is_online(&member.member_id)).collect();
        let mut offline: Vec<_> = server.members.iter().filter(|member| !is_online(&member.member_id)).collect();
        online.sort_by_key(|member| member.name.to_lowercase());
        offline.sort_by_key(|member| member.name.to_lowercase());

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (heading, list, lit) in [("ONLINE", online, true), ("OFFLINE", offline, false)] {
                if list.is_empty() {
                    continue;
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(RichText::new(format!("{heading} — {}", list.len())).color(palette.muted).size(12.0).family(theme::semibold()));
                });
                ui.add_space(2.0);
                for member in list {
                    let (rect, response) = widgets::row(ui, 32.0 + 2.0 * self.prefs.appearance.density.member_pad(), false, &palette);
                    if response.clicked() {
                        self.open_card(Some(member.member_id.clone()), &member.name, rect);
                    }
                    // A right click opens the menu of the person, as `personFromMember` made it.
                    let peer = peers.iter().find(|peer| peer.member_id.as_deref() == Some(member.member_id.as_str()));
                    let mine = member.member_id == my_id;
                    let in_room = !mine
                        && self.room.voice.as_ref().is_some_and(|voice| {
                            voice.space_id == server.id && peer.and_then(|peer| peer.voice.as_ref()).is_some_and(|theirs| theirs.room_id == voice.room_id)
                        });
                    let person = crate::room_view::Person {
                        id: peer.map(|peer| peer.id.clone()).unwrap_or_default(),
                        name: member.name.clone(),
                        member_id: Some(member.member_id.clone()),
                        mic: false,
                        deaf: false,
                        live: false,
                        camera: false,
                        me: mine,
                    };
                    self.person_menu(&response, &person, &server.id, in_room);
                    let avatar = Rect::from_center_size(rect.left_center() + Vec2::new(24.0, 0.0), Vec2::splat(32.0));
                    if !lit {
                        ui.set_opacity(0.45);
                    }
                    let picture = self.pictures.avatar(&self.hub, Some(&member.member_id));
                    widgets::paint_avatar(ui, avatar, &member.name, picture);
                    ui.set_opacity(1.0);
                    widgets::status_dot(ui, avatar, lit, &palette, self.regions.side);
                    let mine = member.member_id == my_id;
                    let label = if mine { format!("{} (você)", member.name) } else { member.name.clone() };
                    // Who is in a room of this server says so under the name.
                    let place = if mine {
                        self.room.voice.as_ref().filter(|voice| voice.space_id == server.id).map(|voice| (voice.room_id.clone(), self.room.streams.outgoing.is_some()))
                    } else {
                        peers.iter().filter(|peer| peer.member_id.as_deref() == Some(member.member_id.as_str())).find_map(|peer| peer.voice.as_ref().map(|voice| (voice.room_id.clone(), voice.live)))
                    };
                    let activity = place.map(|(room_id, live)| {
                        let room = server.channels.iter().find(|channel| channel.id == room_id).map_or("uma sala", |channel| channel.name.as_str());
                        if live { format!("Transmitindo em {room}") } else { format!("Na sala {room}") }
                    });
                    paint_row_name(ui, rect, 50.0, &label, &palette, lit, self.styles.of(Some(&member.member_id)), activity.as_deref());
                }
            }
        });
    }

    fn main(&mut self, ui: &mut Ui) {
        let palette = self.palette;
        let (header, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), self.prefs.appearance.density.header_height()), Sense::hover());
        ui.painter().hline(header.x_range(), header.bottom(), egui::Stroke::new(1.0, palette.line));
        let (title, topic, icon) = match &self.nav {
            Nav::Home => ("Início".to_owned(), String::new(), Some(icons::HOME)),
            Nav::Direct(id) => {
                let name = self.contacts.iter().find(|(contact, _)| &contact.id == id).map_or("Conversa", |(contact, _)| &contact.name);
                (name.to_owned(), String::new(), None)
            }
            Nav::Server { server_id, channel_id } => {
                let server = self.servers.iter().find(|server| &server.id == server_id);
                let channel = server.and_then(|server| server.channels.iter().find(|channel| Some(&channel.id) == channel_id.as_ref()));
                match (server, channel) {
                    (Some(server), Some(channel)) if channel.kind == ChannelKind::Voice => {
                        (channel.name.clone(), format!("{} na sala", self.people_in(&server.id, &channel.id).len()), Some(icons::SPEAKER))
                    }
                    (Some(server), Some(channel)) => (channel.name.clone(), server.name.clone(), Some(icons::HASH)),
                    (Some(server), None) => (server.name.clone(), String::new(), None),
                    _ => ("Telinha".to_owned(), String::new(), None),
                }
            }
        };
        let mut left = header.left_center() + Vec2::new(16.0, 0.0);
        if let Some(icon) = icon {
            icon.image(22.0, palette.muted).paint_at(ui, Rect::from_center_size(left + Vec2::new(11.0, 0.0), Vec2::splat(22.0)));
            left.x += 30.0;
        }
        let used = widgets::paint_text_clipped(ui, left, header.width() * 0.5, &title, FontId::new(16.0, theme::semibold()), palette.strong);
        if !topic.is_empty() {
            let at = left + Vec2::new(used + 10.0, 1.0);
            widgets::paint_text_clipped(ui, at, header.width() * 0.3, &topic, FontId::proportional(13.0), palette.muted);
        }
        if self.current_server().is_some() {
            let mut tools = ui.new_child(egui::UiBuilder::new().max_rect(header.shrink2(Vec2::new(12.0, 8.0))).layout(Layout::right_to_left(Align::Center)));
            let color = if self.prefs.show_members { palette.strong } else { palette.soft };
            if widgets::icon_button(&mut tools, icons::MEMBERS, 32.0, color, palette.hover, "Mostrar ou esconder os membros").clicked() {
                self.prefs.show_members = !self.prefs.show_members;
                self.prefs.save();
            }
            if widgets::icon_button(&mut tools, icons::INVITE, 32.0, palette.soft, palette.hover, "Convidar pessoas").clicked() {
                self.open_invite();
            }
        }
        // The members sit under the header, beside the conversation, as `.main-body` laid them.
        if self.prefs.show_members && self.current_server().is_some() {
            Panel::right("members")
                .exact_size(MEMBERS_WIDTH)
                .resizable(false)
                .show_separator_line(false)
                .frame(if palette.glass {
                    // A darker strip inside the main glass, as `.members` of the sketch.
                    Frame::new().fill(Color32::from_black_alpha(36)).corner_radius(CornerRadius { nw: 0, ne: 0, sw: 0, se: self.corner() }).inner_margin(Margin::same(8))
                } else {
                    Frame::new().fill(self.regions.side).inner_margin(Margin::same(8))
                })
                .show(ui, |ui| {
                    if palette.glass {
                        let strip = ui.max_rect().expand(8.0);
                        ui.painter().vline(strip.left() + 0.5, strip.y_range(), egui::Stroke::new(1.0, palette.line));
                    }
                    self.members(ui);
                });
        }

        let room = match &self.nav {
            Nav::Direct(contact_id) => Some((contact_id.clone(), telinha_core::hub::DM_ROOM.to_owned())),
            Nav::Server { server_id, channel_id: Some(channel_id) } => self
                .servers
                .iter()
                .find(|server| &server.id == server_id)
                .and_then(|server| server.channels.iter().find(|channel| &channel.id == channel_id))
                .filter(|channel| channel.kind == ChannelKind::Voice)
                .map(|channel| (server_id.clone(), channel.id.clone())),
            _ => None,
        };
        if let Some((space_id, room_id)) = room {
            self.room_panel(ui, &space_id, &room_id);
        }
        match self.chrome() {
            Some(chrome) => {
                let hub = Arc::clone(&self.hub);
                match self.chat.show(ui, &hub, &mut self.pictures, &self.styles, &palette, &chrome) {
                    Some(ChatAction::OpenImage { uri, bytes }) => self.lightbox = Lightbox::open(ui.ctx(), uri, bytes),
                    Some(ChatAction::Profile { member_id, name, anchor }) => self.open_card(member_id, &name, anchor),
                    Some(ChatAction::Delete { message_id, ask: true }) => {
                        self.dialog = Some(Dialog::Confirm {
                            danger: Danger::DeleteMessage(message_id),
                            title: "Apagar a mensagem".to_owned(),
                            text: "Ela some daqui e de quem estiver online agora. Quem estiver offline continua com ela.".to_owned(),
                            confirm: "Apagar".to_owned(),
                        });
                    }
                    Some(ChatAction::Delete { message_id, ask: false }) => {
                        let context = ui.ctx().clone();
                        self.apply_danger(&context, Danger::DeleteMessage(message_id));
                    }
                    Some(ChatAction::Notice(text)) => self.notice(text),
                    None => {}
                }
            }
            None if self.nav == Nav::Home => self.home_view(ui),
            None => {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("Escolha uma conversa à esquerda.").color(palette.muted));
                });
            }
        }
    }
}

/// Menus, dialogs and notices.
impl TelinhaApp {
    pub(crate) fn notice(&mut self, text: impl Into<String>) {
        let context = self.context.clone();
        self.toast(&context, text);
    }

    pub fn toast(&mut self, context: &egui::Context, text: impl Into<String>) {
        self.toast = Some((text.into(), context.input(|input| input.time) + 4.0));
    }

    fn show_toast(&mut self, ui: &mut Ui) {
        let Some((text, until)) = &self.toast else { return };
        let now = ui.input(|input| input.time);
        if now >= *until {
            self.toast = None;
            return;
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(until - now));
        let palette = self.palette;
        egui::Area::new(egui::Id::unique("toast"))
            .order(egui::Order::Tooltip)
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -28.0))
            .show(ui.ctx(), |ui| {
                Frame::new()
                    .fill(palette.float)
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.set_max_width(520.0);
                        ui.label(RichText::new(text).color(palette.strong).size(14.0));
                    });
            });
    }

    fn use_chosen_devices(&self) {
        telinha_voice::audio::choose_devices(telinha_voice::audio::DeviceChoice {
            input: self.prefs.input_device.clone(),
            output: self.prefs.output_device.clone(),
        });
        telinha_voice::audio::set_sensitivity(self.prefs.sensitivity_auto, self.prefs.sensitivity_db);
    }

    /// Sets the input sensitivity as the settings would, for the pictures of the tests.
    pub fn sensitivity_for_test(&mut self, auto: bool, threshold: f32) {
        self.prefs.sensitivity_auto = auto;
        self.prefs.sensitivity_db = threshold;
    }

    pub fn open_settings(&mut self) {
        // The tests draw the window without asking Windows what is plugged in.
        let audible = self.room.audible();
        let (inputs, outputs) = if audible { telinha_voice::audio::device_names() } else { (Vec::new(), Vec::new()) };
        let cameras = if audible { telinha_voice::camera::camera_names() } else { Vec::new() };
        self.settings = Some(Settings::new(&self.hub.profile_name(), inputs, outputs, cameras, self.hub.profile_style()));
        self.mic_test = audible.then(telinha_voice::audio::MicTest::start);
        self.stop_preview();
        self.camera_status.clear();
    }

    /// Ends the test of the camera of the settings.
    fn stop_preview(&mut self) {
        self.camera_test = None;
        self.test_feed = None;
        self.previewing = false;
    }

    /// Starts the test of the camera of the settings. While the call has the camera on, the
    /// test shows what the call sends.
    fn start_preview(&mut self) {
        self.camera_status.clear();
        if self.camera_test.is_none() && !self.camera_on() {
            let context = self.context.clone();
            let feed = telinha_voice::video::CameraFeed::new(crate::room_view::video_waker(context));
            self.camera_test = Some(telinha_voice::camera::Camera::start(self.prefs.camera.clone(), self.prefs.blur, feed.sink()));
            self.test_feed = Some(feed);
        }
        self.previewing = true;
    }

    /// What the settings say about the camera being tested, and whether the test still runs.
    fn check_preview(&mut self) {
        if !self.previewing {
            return;
        }
        let camera = self.camera_test.as_ref().or_else(|| self.call_camera());
        let Some(camera) = camera else {
            self.previewing = false;
            return;
        };
        if let Some(problem) = camera.problem() {
            self.camera_status = problem;
            self.stop_preview();
        } else if camera.blur_supported() == Some(false) {
            self.camera_status = "Desfocar o fundo precisa de uma câmera com os efeitos do Windows (Windows Studio Effects).".to_owned();
        } else {
            self.camera_status.clear();
        }
    }

    /// Works out the colors, the fills and the gradient from the choices of the person.
    fn apply_appearance(&mut self) {
        let look = &self.prefs.appearance;
        (self.palette, self.regions) = look.palette(self.system_light);
        self.side = look.side_palette(self.system_light).unwrap_or(self.palette);
        self.backdrop = look.gradient();
        self.restyle = true;
    }

    /// Changes how the app looks and keeps the choice. The size is applied by who calls, at
    /// the moment that does not pull a slider from under the mouse.
    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.prefs.appearance = appearance;
        self.prefs.save();
        self.apply_appearance();
    }

    pub fn appearance(&self) -> &Appearance {
        &self.prefs.appearance
    }

    /// Gives the colors to egui again on the next draw, for a window that was just made.
    pub fn restyle(&mut self) {
        self.restyle = true;
        self.room.forget_textures();
    }

    /// Shows the settings scrolled this far down. For the tests.
    /// Shows a page of the settings. For the tests.
    pub fn settings_page_for_test(&mut self, page: crate::settings::Page) {
        if let Some(settings) = &mut self.settings {
            settings.page_for_test(page);
        }
    }

    pub fn scroll_settings_for_test(&mut self, offset: f32) {
        if let Some(settings) = &mut self.settings {
            settings.scroll_for_test(offset);
        }
    }

    pub fn close_settings(&mut self) {
        self.settings = None;
        self.mic_test = None;
        self.stop_preview();
    }

    /// Opens the editor that picks which part of a picture becomes the avatar or the banner.
    pub fn open_crop(&mut self, bytes: &[u8], banner: bool) {
        match CropEditor::open(bytes, banner) {
            Ok(editor) => self.crop = Some(editor),
            Err(error) => self.notice(error),
        }
    }

    pub fn close_crop(&mut self) {
        self.crop = None;
    }

    /// Opens "Selecione uma imagem" with the last avatars, as `openAvatarPicker`.
    pub fn open_avatar_picker(&mut self) {
        let own = self.hub.member_id();
        let has_avatar = self.pictures.avatar(&self.hub, Some(&own)).is_some();
        self.avatar_picker = Some(crate::avatar_picker::AvatarPicker::new(self.hub.recent_avatars(), has_avatar));
    }

    pub fn close_avatar_picker(&mut self) {
        self.avatar_picker = None;
    }

    fn show_avatar_picker(&mut self, context: &egui::Context) {
        use crate::avatar_picker::PickerAnswer;
        let Some(picker) = &mut self.avatar_picker else { return };
        match picker.show(context, &self.palette) {
            PickerAnswer::Open => {}
            PickerAnswer::Close => self.avatar_picker = None,
            PickerAnswer::Picked(bytes) => {
                self.avatar_picker = None;
                self.open_crop(&bytes, false);
            }
            PickerAnswer::Recent(hash) => {
                self.avatar_picker = None;
                self.hub.use_recent_avatar(&hash);
                self.pictures.forget_own();
            }
            PickerAnswer::Remove => {
                self.avatar_picker = None;
                self.hub.remove_picture(false);
                self.pictures.forget_own();
            }
        }
    }

    fn show_crop(&mut self, context: &egui::Context) {
        let Some(editor) = &mut self.crop else { return };
        match editor.show(context, &self.palette) {
            CropAnswer::Open => {}
            CropAnswer::Cancel => self.crop = None,
            CropAnswer::Failed(error) => self.notice(error),
            CropAnswer::Save { banner, data } => {
                self.crop = None;
                if self.hub.set_picture(banner, "image/webp", data) {
                    self.pictures.forget_own();
                    if !banner {
                        self.notice("Avatar atualizado.");
                    }
                } else {
                    self.notice("Não consegui usar essa imagem.");
                }
            }
        }
    }

    fn show_settings(&mut self, context: &egui::Context) {
        let Some(mut settings) = self.settings.take() else { return };
        let own = self.hub.member_id();
        let avatar = self.pictures.avatar(&self.hub, Some(&own));
        let banner = self.pictures.banner(&self.hub, Some(&own));
        let name = self.hub.profile_name();
        let devices = (self.prefs.input_device.clone(), self.prefs.output_device.clone());
        let activity = self.activity_of(Some(&own));
        let appearance = self.prefs.appearance.clone();
        self.check_preview();
        let app_icon = self.prefs.app_icon.clone();
        let mic_level = self.mic_test.as_ref().map_or(0.0, telinha_voice::audio::MicTest::level);
        let mic_db = self.mic_test.as_ref().map_or(*telinha_voice::audio::SENSITIVITY_RANGE.start(), telinha_voice::audio::MicTest::decibels);
        let (gate_open, gate_threshold) = self.mic_test.as_ref().map_or((false, -50.0), telinha_voice::audio::MicTest::gate);
        let mic_status = self.mic_test.as_ref().and_then(telinha_voice::audio::MicTest::problem).unwrap_or_else(|| "".to_owned());
        if self.mic_test.is_some() {
            // The bar moves as `startMicTest` moved it, every 80 milliseconds.
            context.request_repaint_after(std::time::Duration::from_millis(80));
        }
        let preview = if self.previewing {
            let feed = if self.camera_test.is_some() { self.test_feed.clone() } else { self.room.camera_feed.clone() };
            feed.and_then(|feed| self.video_texture(context, "settings:camera", feed.preview(), egui::vec2(362.0, 204.0)))
        } else {
            None
        };
        let camera = self.prefs.camera.clone();
        let camera_status = self.camera_status.clone();
        let update = self.update_info();
        if update.busy {
            context.request_repaint_after(std::time::Duration::from_millis(300));
        }
        let view = SettingsView {
            name: &name,
            activity: &activity,
            avatar,
            banner,
            appearance: &appearance,
            version: env!("TELINHA_VERSION"),
            input: devices.0.as_deref(),
            output: devices.1.as_deref(),
            mic_level,
            mic_status: &mic_status,
            mic_db,
            gate_open,
            gate_threshold,
            sensitivity_auto: self.prefs.sensitivity_auto,
            sensitivity_db: self.prefs.sensitivity_db,
            app_icon: &app_icon,
            camera: camera.as_deref(),
            blur: self.prefs.blur,
            previewing: self.previewing,
            preview,
            camera_status: &camera_status,
            update,
        };
        let (actions, open) = settings.show(context, &self.palette, view);
        if open {
            self.settings = Some(settings);
        } else {
            self.mic_test = None;
            self.stop_preview();
        }
        for action in actions {
            match action {
                SettingsAction::Name(name) => self.hub.set_name(&name),
                SettingsAction::Crop { banner, bytes } => self.open_crop(&bytes, banner),
                SettingsAction::AvatarPicker => self.open_avatar_picker(),
                SettingsAction::RemovePicture { banner } => {
                    self.hub.remove_picture(banner);
                    self.pictures.forget_own();
                }
                SettingsAction::Appearance(appearance) => {
                    // The size takes hold here, when the slider was let go.
                    if (appearance.zoom - self.prefs.appearance.zoom).abs() > 0.01 {
                        context.set_zoom_factor(appearance.zoom / 100.0);
                    }
                    self.set_appearance(*appearance);
                }
                SettingsAction::Style(style) => {
                    self.hub.set_style(&style);
                    self.remember_own_style();
                }
                SettingsAction::Devices { input, output } => {
                    self.prefs.input_device = input;
                    self.prefs.output_device = output;
                    self.prefs.save();
                    self.use_chosen_devices();
                    self.restart_call();
                    if self.mic_test.take().is_some() {
                        self.mic_test = Some(telinha_voice::audio::MicTest::start());
                    }
                }
                SettingsAction::Sensitivity { auto, threshold } => {
                    // The gate reads it on its next piece of sound: the call goes on as it is.
                    self.prefs.sensitivity_auto = auto;
                    self.prefs.sensitivity_db = threshold;
                    self.prefs.save();
                    telinha_voice::audio::set_sensitivity(auto, threshold);
                }
                SettingsAction::AppIcon(id) => {
                    crate::app_icons::apply(context, &id);
                    self.prefs.app_icon = id;
                    self.prefs.save();
                }
                SettingsAction::Camera(camera) => {
                    self.prefs.camera = camera;
                    self.prefs.save();
                    // As `setDevice`: the camera on in the call and the test change with it.
                    self.restart_camera();
                    if self.camera_test.take().is_some() {
                        self.start_preview();
                    }
                }
                SettingsAction::Blur(on) => {
                    self.prefs.blur = on;
                    self.prefs.save();
                    for camera in self.camera_test.iter().chain(self.call_camera()) {
                        camera.set_blur(on);
                    }
                }
                SettingsAction::Preview(true) => self.start_preview(),
                SettingsAction::Preview(false) => self.stop_preview(),
                SettingsAction::ShowLog => {
                    let folder = self.hub.data_dir().join("logs");
                    if std::process::Command::new("explorer.exe").arg(&folder).spawn().is_err() {
                        self.notice(format!("O registro fica em {}", folder.display()));
                    }
                }
                SettingsAction::CheckUpdates => {
                    if let Some(updater) = self.updater() {
                        let waker = context.clone();
                        self.runtime.spawn(async move {
                            updater.check_now().await;
                            waker.request_repaint();
                        });
                        context.request_repaint_after(std::time::Duration::from_millis(100));
                    }
                }
                SettingsAction::Notice(text) => self.notice(text),
            }
        }
    }

    fn open_invite(&mut self) {
        let Some(server) = self.current_server() else { return };
        if let Some(code) = self.hub.invite_code(&server.id) {
            self.dialog = Some(Dialog::Invite { server_name: server.name.clone(), code, feedback: String::new(), sent: Vec::new() });
        }
    }

    fn server_menu(&mut self, header: &egui::Response, server: &Server) {
        let palette = self.palette;
        let frame = if palette.glass {
            Frame::new().inner_margin(Margin::same(6))
        } else {
            Frame::new()
                .fill(palette.float)
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::same(6))
                .shadow(egui::Shadow { offset: [0, 8], blur: 16, spread: 0, color: Color32::from_black_alpha(102) })
        };
        let anchor = header.rect.shrink2(Vec2::new(10.0, 0.0));
        egui::Popup::menu(header).anchor(anchor).gap(4.0).frame(frame).width(anchor.width() - 12.0).style(|style: &mut egui::Style| style.spacing.item_spacing = Vec2::ZERO).show(|ui| crate::glass::menu_contents(ui, Margin::same(6), |ui| {
            if server_menu_item(ui, &palette, "Convidar pessoas", false) {
                self.open_invite();
            }
            if server_menu_item(ui, &palette, "Criar canal de texto", false) {
                self.dialog = Some(Dialog::channel(&server.id, None, ChannelKind::Text));
            }
            if server_menu_item(ui, &palette, "Criar canal de voz", false) {
                self.dialog = Some(Dialog::channel(&server.id, None, ChannelKind::Voice));
            }
            if server_menu_item(ui, &palette, "Renomear servidor", false) {
                self.dialog = Some(Dialog::for_purpose(Purpose::RenameServer(server.id.clone()), &server.name));
            }
            if server_menu_item(ui, &palette, "Relay das chamadas", false) {
                let current = server.relay.as_ref().map_or_else(String::new, |relay| format!("{} {} {}", relay.urls.join(","), relay.username, relay.credential));
                self.dialog = Some(Dialog::for_purpose(Purpose::SetRelay(server.id.clone()), current.trim()));
            }
            if server_menu_item(ui, &palette, "Sair do servidor", true) {
                self.dialog = Some(Dialog::Confirm {
                    danger: Danger::LeaveServer(server.id.clone()),
                    title: "Sair do servidor".to_owned(),
                    text: format!("Sair de {}? As conversas guardadas neste computador somem e você só volta com um convite novo.", server.name),
                    confirm: "OK".to_owned(),
                });
            }
        }));
    }

    fn show_dialog(&mut self, context: &egui::Context) {
        let contacts: Vec<InviteContact> = if matches!(self.dialog, Some(Dialog::Invite { .. })) {
            let list: Vec<_> = self.contacts.clone();
            list.into_iter()
                .map(|(contact, online)| {
                    let member = self.hub.member_of_contact(&contact.id);
                    let picture = self.pictures.avatar(&self.hub, member.as_deref());
                    InviteContact { id: contact.id, name: contact.name, online, picture }
                })
                .collect()
        } else {
            Vec::new()
        };
        let Some(dialog) = &mut self.dialog else { return };
        match dialog.show(context, &self.palette, &contacts) {
            Answer::Open => {}
            Answer::Close => self.dialog = None,
            Answer::CreateServer(name) => {
                let name = name.trim();
                if name.is_empty() {
                    self.dialog_error("Escreva um nome.");
                    return;
                }
                let server = self.hub.create_server(name);
                self.servers = self.hub.servers();
                self.dialog = None;
                self.navigate(Nav::Server { server_id: server.id.clone(), channel_id: first_text_channel(&server) });
                self.open_invite();
            }
            Answer::JoinServer(code) => match self.hub.join_server(code.trim()) {
                Some((server, created)) => {
                    self.servers = self.hub.servers();
                    self.dialog = None;
                    self.navigate(Nav::Server { server_id: server.id.clone(), channel_id: first_text_channel(&server) });
                    let text = if created {
                        format!("Você entrou em {}. Os canais aparecem quando alguém de lá estiver online.", server.name)
                    } else {
                        format!("Você já está em {}.", server.name)
                    };
                    self.toast(context, text);
                }
                None => self.dialog_error("Esse convite não é válido. Copie o texto inteiro, que começa com TELINHA-GRUPO."),
            },
            Answer::SaveChannel { server_id, channel_id, kind, name } => {
                self.dialog = None;
                match channel_id {
                    Some(channel_id) => {
                        self.hub.rename_channel(&server_id, &channel_id, name.trim());
                    }
                    None => match self.hub.add_channel(&server_id, kind, name.trim()) {
                        Some(channel) => {
                            self.servers = self.hub.servers();
                            self.navigate(Nav::Server { server_id, channel_id: Some(channel.id) });
                        }
                        None => self.toast(context, "Esse servidor já tem canais demais."),
                    },
                }
            }
            Answer::RemoveChannel(server_id, channel_id) => {
                let name = self.hub.server(&server_id).and_then(|server| server.channels.into_iter().find(|channel| channel.id == channel_id)).map(|channel| channel.name);
                self.dialog = name.map(|name| Dialog::Confirm {
                    danger: Danger::RemoveChannel(server_id, channel_id),
                    title: "Excluir canal".to_owned(),
                    text: format!("Excluir {name} para todo mundo do servidor? As mensagens guardadas neste computador somem."),
                    confirm: "OK".to_owned(),
                });
            }
            Answer::SendInvite(contact_id) => self.send_invite(&contact_id),
            Answer::Danger(purpose) => {
                self.dialog = None;
                match purpose {
                    Purpose::Contact(contact_id) => {
                        let name = self.contacts.iter().find(|(contact, _)| contact.id == contact_id).map(|(contact, _)| contact.name.clone()).unwrap_or_default();
                        self.dialog = Some(Dialog::Confirm {
                            danger: Danger::RemoveContact(contact_id),
                            title: "Remover contato".to_owned(),
                            text: format!("Remover {name} dos contatos? A conversa guardada também some."),
                            confirm: "OK".to_owned(),
                        });
                    }
                    Purpose::SetRelay(server_id) => {
                        self.hub.set_server_relay(&server_id, None);
                    }
                    _ => {}
                }
            }
            Answer::Text(purpose, text) => match self.apply_text(&purpose, text.trim()) {
                Ok(()) => self.dialog = None,
                Err(error) => self.dialog_error(&error),
            },
            Answer::Confirmed(danger) => {
                self.dialog = None;
                self.apply_danger(context, danger);
            }
        }
    }

    fn dialog_error(&mut self, text: &str) {
        if let Some(dialog) = &mut self.dialog {
            dialog.set_error(text);
        }
    }

    /// Sends the invite of the open server to a contact as a message, as the old app did.
    fn send_invite(&mut self, contact_id: &str) {
        let Some(Dialog::Invite { server_name, code, .. }) = &self.dialog else { return };
        let text = format!("Entre no meu servidor {server_name} no Telinha: {code}");
        let conversation = Conversation::direct(contact_id);
        let outgoing = telinha_core::hub::Outgoing { text, ..Default::default() };
        let label = match self.hub.send_chat(&conversation, outgoing) {
            Ok(Some(message)) => {
                let label = if message.delivered == Some(false) { "Sem conexão" } else { "Enviado" };
                self.chat.received(&conversation, message);
                label
            }
            Ok(None) => "Enviado",
            Err(error) => {
                self.notice(format!("Não consegui enviar: {error}."));
                return;
            }
        };
        if let Some(dialog) = &mut self.dialog {
            dialog.invite_sent(contact_id, label);
        }
    }

    fn apply_text(&mut self, purpose: &Purpose, text: &str) -> Result<(), String> {
        let empty = || "Escreva um nome.".to_owned();
        match purpose {
            Purpose::RenameServer(server_id) => {
                if text.is_empty() {
                    return Err(empty());
                }
                self.hub.rename_server(server_id, text);
            }
            Purpose::Contact(contact_id) => {
                if text.is_empty() {
                    return Err(empty());
                }
                self.hub.rename_contact(contact_id, text);
            }
            Purpose::RenameSound(sound_id) => {
                if text.is_empty() {
                    return Err(empty());
                }
                self.hub.rename_sound(sound_id, text);
            }
            Purpose::NameSound => self.keep_picked_sound(text)?,
            Purpose::SetRelay(server_id) => {
                // As the old app read it: the addresses, then the user and the password.
                let mut parts = text.split_whitespace();
                let urls = parts.next().unwrap_or("");
                let relay = serde_json::json!({ "urls": urls, "username": parts.next().unwrap_or(""), "credential": parts.next().unwrap_or("") });
                let wanted = !urls.is_empty();
                self.hub.set_server_relay(server_id, wanted.then_some(&relay));
                let kept = self.hub.server(server_id).is_some_and(|server| server.relay.is_some());
                if wanted && !kept {
                    return Err("Nenhum endereço válido. Um relay começa com turn: ou turns:, como turn:exemplo.com:3478.".to_owned());
                }
            }
            Purpose::ChangeName => {
                if text.is_empty() {
                    return Err(empty());
                }
                self.hub.set_name(text);
            }
        }
        Ok(())
    }

    fn apply_danger(&mut self, context: &egui::Context, danger: Danger) {
        match danger {
            Danger::LeaveServer(server_id) => {
                self.hub.leave_server(&server_id);
                self.servers = self.hub.servers();
                self.navigate(Nav::Home);
            }
            Danger::RemoveChannel(server_id, channel_id) => {
                self.hub.remove_channel(&server_id, &channel_id);
            }
            Danger::RemoveContact(contact_id) => {
                self.hub.remove_contact(&contact_id);
                self.contacts = self.hub.contacts();
                self.navigate(Nav::Home);
            }
            Danger::DeleteMessage(message_id) => {
                let Some(conversation) = self.conversation() else { return };
                if self.hub.delete_message(&conversation, &message_id) == Some(false) {
                    self.toast(context, "Apagada aqui. Sem conexão, os outros continuam com ela.");
                }
            }
        }
    }

    /// Opens a dialog from outside, for the tests.
    pub fn open_dialog(&mut self, dialog: Dialog) {
        self.dialog = Some(dialog);
    }

    pub fn close_dialog(&mut self) {
        self.dialog = None;
    }

    /// Opens a picture over the window, for the tests.
    pub fn open_picture(&mut self, uri: &str, bytes: Vec<u8>) {
        self.lightbox = Lightbox::open(&self.context, uri.to_owned(), bytes.into());
    }

    /// Shows a made-up link preview above the box of the open conversation. For the tests.
    pub fn draft_for_test(&mut self, text: &str) {
        self.chat.draft_for_test(text);
    }

    pub fn sharp_glass_for_test(&mut self, sharp: bool) {
        self.sharp_glass = sharp;
    }

    pub fn composer_height_for_test(&self) -> f32 {
        self.chat.composer_height_for_test()
    }

    pub fn scroll_chat_for_test(&mut self, offset: f32) {
        self.chat.scroll_for_test(offset);
    }

    pub fn link_for_test(&mut self, url: &str, site: &str, title: &str, description: &str) {
        self.chat.link_for_test(url, site, title, description);
    }

    pub fn close_picture(&mut self) {
        self.lightbox = None;
    }
}

impl eframe::App for TelinhaApp {
    /// Runs before every frame and also while the window is minimized, when `ui` does not.
    /// The hub and the call must keep being heard then: a minimized app that stops taking
    /// events keeps calling people who already left and never sees who came back.
    fn logic(&mut self, _context: &egui::Context, _frame: &mut eframe::Frame) {
        let _runtime = self.runtime.enter();
        self.drain_events();
        self.drain_tasks();
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        TelinhaApp::ui(self, ui);
    }
}

pub(crate) fn first_text_channel(server: &Server) -> Option<String> {
    server.channels.iter().find(|channel| channel.kind == ChannelKind::Text).map(|channel| channel.id.clone())
}

/// Up to two letters for the button of a server: the first letter of its first two words.
pub fn server_initials(name: &str) -> String {
    name.split_whitespace().take(2).filter_map(|word| word.chars().next()).flat_map(char::to_uppercase).collect()
}

enum RailFace {
    Icon(icons::Icon),
    Text(String),
    Add,
}

/// The room around the card of the call and the person at the foot of the sidebar.
const FOOT_MARGIN: f32 = 8.0;

fn rail_button(ui: &mut Ui, palette: &Palette, selected: bool, waiting: bool, face: RailFace, tip: &str) -> egui::Response {
    let (slot, response) = ui.allocate_exact_size(Vec2::new(RAIL_WIDTH, 48.0), Sense::click());
    let rect = Rect::from_center_size(slot.center(), Vec2::splat(48.0));
    let lit_now = selected || response.hovered();
    // The circle melts into a rounded square and the mark at the edge grows, both eased.
    let lit = crate::motion::ease(ui.ctx(), response.id.with("rail-lit"), lit_now, crate::motion::HOVER + 0.04);
    let add = matches!(face, RailFace::Add);
    let on = if add { palette.green } else { palette.accent };
    let fill = crate::motion::mix(palette.main, on, lit);
    let round = CornerRadius::same(crate::motion::lerp(24.0, 16.0, lit).round() as u8);
    if !palette.glass {
        ui.painter().rect_filled(rect, round, fill);
    } else {
        if lit < 1.0 {
            let pane = crate::glass::Pane { shine: false, ..crate::glass::Pane::new(Color32::from_white_alpha(20), f32::from(round.nw)) };
            crate::glass::pane(ui.painter(), rect, &pane);
        }
        if lit > 0.0 {
            if add {
                ui.painter().rect_filled(rect, round, palette.green.gamma_multiply(lit));
            } else {
                // Lit glass buttons take the gradient of the accent.
                crate::glass::accent_fill(ui.painter(), egui::Shape::rect_filled(rect, round, Color32::WHITE.gamma_multiply(lit)));
            }
        }
    }
    // The mark at the edge: tall for where the person is, short for messages waiting to be read.
    let target = if selected { 40.0 } else if response.hovered() { 20.0 } else if waiting { 8.0 } else { 0.0 };
    let height = ui.ctx().animate_value_with_time(response.id.with("rail-mark"), target, crate::motion::STATE);
    if height > 0.5 {
        let mark = Rect::from_center_size(egui::pos2(slot.left() + 2.0, slot.center().y), Vec2::new(8.0, height));
        ui.painter().rect_filled(mark, CornerRadius::same(4), palette.strong);
    }
    let rest = if add { palette.green } else { palette.text };
    let ink = crate::motion::mix(rest, Color32::WHITE, lit);
    match face {
        RailFace::Icon(icon) => icon.image(28.0, ink).paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(28.0))),
        RailFace::Add => icons::PLUS.image(24.0, ink).paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(24.0))),
        RailFace::Text(text) => {
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::new(16.0, theme::semibold()), ink);
        }
    }
    response.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The title of a group of rows. True when its plus button was clicked.
fn section_heading(ui: &mut Ui, palette: &Palette, text: &str, with_add: bool) -> bool {
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
    let mut clicked = false;
    ui.painter().text(
        rect.left_center() + Vec2::new(8.0, 0.0),
        Align2::LEFT_CENTER,
        text,
        FontId::new(12.0, theme::semibold()),
        palette.muted,
    );
    if with_add {
        let plus = Rect::from_center_size(rect.right_center() - Vec2::new(16.0, 0.0), Vec2::splat(24.0));
        let response = ui.interact(plus, ui.scope_id().with(("add", text)), Sense::click());
        let ink = if response.hovered() { palette.strong } else { palette.muted };
        icons::PLUS.image(16.0, ink).paint_at(ui, plus.shrink(4.0));
        clicked = response.on_hover_text("Criar canal").on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
    }
    ui.add_space(2.0);
    clicked
}

/// The frame of a right-click menu, as `.context-menu` drew it.
pub(crate) fn menu_frame(palette: &Palette) -> Frame {
    // On glass the pane is drawn by `glass::menu_contents`, under what the menu holds.
    if palette.glass {
        return Frame::new().inner_margin(Margin::symmetric(8, 6));
    }
    Frame::new()
        .fill(palette.float)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 6))
        .shadow(egui::Shadow { offset: [0, 8], blur: 16, spread: 0, color: Color32::from_black_alpha(115) })
}

/// A right-click menu on a widget, at least 230 pixels wide with its padding.
pub(crate) fn context_menu<R>(response: &egui::Response, palette: &Palette, contents: impl FnOnce(&mut Ui) -> R) -> Option<egui::InnerResponse<R>> {
    let frame = menu_frame(palette);
    let margin = frame.inner_margin;
    egui::Popup::context_menu(response)
        .frame(frame)
        .width(214.0)
        .style(|style: &mut egui::Style| style.spacing.item_spacing = Vec2::ZERO)
        .show(|ui| crate::glass::menu_contents(ui, margin, contents))
}

/// The name on top of a context menu, as `.menu-header` drew it.
pub(crate) fn menu_header(ui: &mut Ui, palette: &Palette, label: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0 + 19.0 + 8.0), Sense::hover());
    let room = rect.width() - 16.0;
    widgets::paint_text_clipped(ui, egui::pos2(rect.left() + 8.0, rect.top() + 6.0 + 9.5), room, label, FontId::new(14.0, theme::semibold()), palette.strong);
}

/// The thin line between groups of a context menu, as `.menu-separator`.
pub(crate) fn menu_separator(ui: &mut Ui, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 9.0), Sense::hover());
    ui.painter().rect_filled(Rect::from_min_size(rect.min + Vec2::new(4.0, 4.0), Vec2::new(rect.width() - 8.0, 1.0)), CornerRadius::ZERO, palette.line_soft);
}

/// One line of a context menu, as `.menu-action`. True when it was chosen, which also closes the menu.
pub(crate) fn menu_item(ui: &mut Ui, palette: &Palette, label: &str, danger: bool) -> bool {
    menu_line(ui, palette, label, danger, 33.0, 8.0)
}

/// One line of the server menu, as `.menu button`: a little taller than a context menu's.
fn server_menu_item(ui: &mut Ui, palette: &Palette, label: &str, danger: bool) -> bool {
    menu_line(ui, palette, label, danger, 35.0, 10.0)
}

fn menu_line(ui: &mut Ui, palette: &Palette, label: &str, danger: bool, height: f32, inset: f32) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let ink = if danger { palette.danger_text } else { palette.text };
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(3), if danger { palette.danger } else { palette.accent });
    }
    let ink = if response.hovered() { Color32::WHITE } else { ink };
    widgets::paint_text_clipped(ui, rect.left_center() + Vec2::new(inset, 0.0), rect.width() - 2.0 * inset, label, FontId::proportional(14.0), ink);
    let clicked = response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// A line of a context menu with a box at the right, as `.menu-check`. True when it was clicked;
/// the menu stays open, as it did.
pub(crate) fn menu_check(ui: &mut Ui, palette: &Palette, label: &str, checked: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 33.0), Sense::click());
    let hovered = response.hovered();
    if hovered {
        ui.painter().rect_filled(rect, CornerRadius::same(3), palette.accent);
    }
    let ink = if hovered { Color32::WHITE } else { palette.text };
    widgets::paint_text_clipped(ui, rect.left_center() + Vec2::new(8.0, 0.0), rect.width() - 46.0, label, FontId::proportional(14.0), ink);
    let others = Rect::from_center_size(egui::pos2(rect.right() - 8.0 - 9.0, rect.center().y), Vec2::splat(18.0));
    if checked {
        // A blue box with a ring of the menu's color inside its border.
        ui.painter().rect_filled(others, CornerRadius::same(4), palette.accent);
        ui.painter().rect_filled(others.shrink(2.0), CornerRadius::same(2), palette.float);
        ui.painter().rect_filled(others.shrink(5.0), CornerRadius::ZERO, palette.accent);
    } else {
        let border = if hovered { Color32::WHITE } else { palette.muted };
        ui.painter().rect_stroke(others.shrink(1.0), CornerRadius::same(3), egui::Stroke::new(2.0, border), egui::StrokeKind::Middle);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A volume in a context menu, as `.menu-slider`: the name and the percent on top, the bar
/// below. Gives the response of the bar.
pub(crate) fn menu_slider(ui: &mut Ui, palette: &Palette, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>) -> egui::Response {
    let width = ui.available_width();
    let (head, head_response) = ui.allocate_exact_size(Vec2::new(width, 7.0 + 19.0), Sense::hover());
    head_response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, label));
    let baseline = head.top() + 7.0 + 9.5;
    ui.painter().text(egui::pos2(head.left() + 8.0, baseline), Align2::LEFT_CENTER, label, FontId::proportional(14.0), palette.text);
    ui.painter().text(egui::pos2(head.right() - 8.0, baseline), Align2::RIGHT_CENTER, format!("{:.0}%", value), FontId::proportional(12.0), palette.muted);
    ui.add_space(6.0);
    let response = ui
        .horizontal(|ui| {
            ui.add_space(8.0);
            ui.spacing_mut().slider_width = width - 16.0;
            ui.style_mut().visuals.selection.bg_fill = palette.accent;
            ui.style_mut().visuals.widgets.inactive.bg_fill = palette.line;
            ui.add(egui::Slider::new(value, range).show_value(false).trailing_fill(true).handle_shape(egui::style::HandleShape::Circle))
        })
        .inner;
    ui.add_space(7.0);
    response.labelled_by(head_response.id)
}
