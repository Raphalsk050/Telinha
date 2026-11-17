//! Draws the window with made-up data and saves pictures of it, without opening anything on
//! screen and without touching the real profile. The pictures go to `client/target/snapshots`.
//!
//!   cargo test -p telinha --test snapshots -- --nocapture

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use egui_kittest::Harness;
use serde_json::{Value, json};
use telinha::app::{Nav, TelinhaApp};
use telinha_core::hub::Hub;
use telinha_core::servers::ChannelKind;
use telinha_core::signaling::default_brokers;
use telinha_core::text::{encode_base64, now_ms};

const KEEP: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const NANDA: &str = "cccccccccccccccccccccccccccccccc";
const PEDRO: &str = "dddddddddddddddddddddddddddddddd";

struct Scene {
    _folder: tempfile::TempDir,
    runtime: tokio::runtime::Runtime,
    hub: Arc<Hub>,
    app: TelinhaApp,
    server_id: String,
    text_channel: String,
    voice_channel: String,
}

fn png(width: u32, height: u32, hue: u8) -> Vec<u8> {
    let picture = image::RgbImage::from_fn(width, height, |x, y| {
        let grid = x % 80 < 2 || y % 80 < 2;
        if grid {
            image::Rgb([255, 255, 255])
        } else {
            image::Rgb([hue, (x * 255 / width) as u8, (y * 255 / height) as u8])
        }
    });
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(picture)
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    bytes
}

fn presence(from: &str, name: &str, member_id: &str, extra: Value) -> Value {
    let mut message = json!({
        "type": "presence", "online": true, "from": from, "at": now_ms(), "mid": telinha_core::text::new_uuid(),
        "name": name, "memberId": member_id,
    });
    message.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    message
}

fn scene() -> Scene {
    let folder = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let guard = runtime.enter();
    let (hub, events) = Hub::open(folder.path(), default_brokers());
    hub.set_name("Raphalsk");
    let server = hub.create_server("Servidor de rapha");
    let other = hub.create_server("Hesperides");
    let _ = other;
    let text_channel = server.channels.iter().find(|channel| channel.kind == ChannelKind::Text).unwrap().id.clone();
    let voice_channel = server.channels.iter().find(|channel| channel.kind == ChannelKind::Voice).unwrap().id.clone();
    hub.add_channel(&server.id, ChannelKind::Text, "canal dos memes");
    hub.add_channel(&server.id, ChannelKind::Voice, "Jogatina");

    let signaling = hub.signaling();
    signaling.loopback(&server.id, &presence("peer-keep", "Keep", KEEP, json!({ "style": { "name": { "effect": "prism", "colors": ["#5865f2", "#eb459e", "#f0b232"] }, "theme": ["#5865f2", "#eb459e"], "bio": "Jogo de tudo um pouco.\nChama no canal de voz." } })));
    signaling.loopback(&server.id, &presence("peer-nanda", "nanda", NANDA, json!({ "style": { "name": { "effect": "neon", "colors": ["#19b8b0"] } } })));
    signaling.loopback(&server.id, &presence("peer-pedro", "pedro", PEDRO, json!({})));
    signaling.loopback(&server.id, &json!({ "type": "presence", "online": false, "from": "peer-pedro", "at": now_ms(), "mid": "bye-pedro" }));

    let chat = |id: &str, author: &str, member: &str, minutes_ago: i64, extra: Value| {
        let mut message = json!({
            "type": "chat", "from": format!("peer-{}", author.to_lowercase()), "at": now_ms(), "mid": format!("mid-{id}"),
            "messageId": id, "channelId": text_channel, "author": author, "memberId": member,
            "sentAt": now_ms() - minutes_ago * 60_000, "text": "",
        });
        message.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        signaling.loopback(&server.id, &message);
    };
    chat("m1", "Keep", KEEP, 50, json!({ "text": "alguém vai jogar hoje?" }));
    chat("m2", "Keep", KEEP, 49, json!({ "text": "to devendo o agiota, preciso de distração" }));
    // Two answers in the thread of the first message, which stay out of the conversation.
    chat("t1", "nanda", NANDA, 48, json!({ "text": "eu jogo! só depois das 21h", "threadId": "m1" }));
    chat("t2", "pedro", PEDRO, 46, json!({ "text": "to dentro, chama no canal de voz", "threadId": "m1" }));
    chat("m3", "nanda", NANDA, 30, json!({ "text": "eu vou, só terminar aqui. olha isso https://example.com/receita" ,
        "embed": { "url": "https://example.com/receita", "title": "Receita de bolo de cenoura", "description": "Fofinho, com cobertura de chocolate, pronto em 40 minutos.", "siteName": "example.com" } }));
    chat("m4", "Keep", KEEP, 12, json!({ "text": "olha essa",
        "image": { "mime": "image/png", "width": 640, "height": 360, "data": encode_base64(&png(640, 360, 200)) } }));
    chat("m5", "nanda", NANDA, 3, json!({ "text": "kkkkk\nboa" }));
    chat("m5v", "Keep", KEEP, 2, json!({ "text": "assiste isso https://www.youtube.com/watch?v=aqz-KE-bpKQ",
        "embed": { "url": "https://www.youtube.com/watch?v=aqz-KE-bpKQ", "title": "Big Buck Bunny 60fps 4K", "description": "", "siteName": "YouTube",
            "image": { "mime": "image/png", "width": 480, "height": 360, "data": encode_base64(&png(480, 360, 90)) } } }));
    chat("m6", "Keep", KEEP, 2, json!({ "text": "chamou a atenção!", "nudge": true }));
    // nanda pinned the recipe.
    signaling.loopback(&server.id, &json!({ "type": "chat-pin", "from": "peer-nanda", "at": now_ms(), "mid": "pin-m3", "messageId": "m3", "channelId": text_channel, "pinned": true, "by": "nanda" }));

    runtime.block_on(async { tokio::time::sleep(Duration::from_millis(150)).await });
    drop(guard);

    let context = egui::Context::default();
    let mut app = TelinhaApp::new(&context, Arc::clone(&hub), events, runtime.handle().clone());
    // Nothing here connects to anyone or makes a sound on the speakers.
    app.disable_calls();
    Scene { _folder: folder, runtime, hub, app, server_id: server.id, text_channel, voice_channel }
}

fn shoot(scene: &mut Scene, name: &str) -> image::RgbaImage {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target").join("snapshots");
    std::fs::create_dir_all(&folder).unwrap();
    shoot_app(&mut scene.app, name)
}

fn shoot_app(app: &mut TelinhaApp, name: &str) -> image::RgbaImage {
    shoot_clicking(app, name, &[])
}

/// Takes the picture after clicking at each place, with the pointer left on the last one.
fn shoot_clicking(app: &mut TelinhaApp, name: &str, clicks: &[(egui::Pos2, egui::PointerButton)]) -> image::RgbaImage {
    shoot_with(app, name, clicks, None)
}

/// Takes the picture with the pointer resting on `pos`, after its hover has eased in.
fn shoot_hovering(app: &mut TelinhaApp, name: &str, pos: egui::Pos2) -> image::RgbaImage {
    shoot_with(app, name, &[], Some(pos))
}

fn shoot_with(app: &mut TelinhaApp, name: &str, clicks: &[(egui::Pos2, egui::PointerButton)], hover: Option<egui::Pos2>) -> image::RgbaImage {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target").join("snapshots");
    // Every picture is drawn in a window of its own, which starts without the app's colors.
    app.restyle();
    // The glass of the glass theme blurs on the same GPU that draws the picture.
    let render = egui_kittest::wgpu::create_render_state(egui_kittest::wgpu::default_wgpu_setup(), eframe::egui_wgpu::RendererOptions::PREDICTABLE);
    let blur = render.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1360.0, 860.0))
        .with_pixels_per_point(1.0)
        .with_max_steps(12)
        .with_context_setup(move |context| {
            egui_extras::install_image_loaders(context);
            telinha::theme::install_fonts(context);
            telinha::theme::apply(context, &telinha::theme::Palette::GRAY);
            telinha::glass::install(context, blur);
        })
        .renderer(egui_kittest::wgpu::WgpuTestRenderer::from_render_state(render))
        // The whole window, without the margin kittest puts around the ui, as in the app.
        .build_ui(|ui| {
            let full = ui.ctx().content_rect();
            let mut window = ui.new_child(egui::UiBuilder::new().max_rect(full));
            window.set_clip_rect(full);
            app.ui(&mut window);
        });
    harness.run_steps(6);
    for (pos, button) in clicks {
        harness.hover_at(*pos);
        harness.run_steps(1);
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton { pos: *pos, button: *button, pressed, modifiers: egui::Modifiers::NONE });
            harness.run_steps(1);
        }
        harness.run_steps(2);
    }
    if let Some(pos) = hover {
        harness.hover_at(pos);
        harness.run_steps(40);
    }
    // The lightbox waits a moment for a picture of the window, which no test window gives.
    std::thread::sleep(Duration::from_millis(300));
    harness.run_steps(3);
    let picture = harness.render().expect("the window renders");
    picture.save(folder.join(format!("{name}.png"))).unwrap();
    println!("saved {}", folder.join(format!("{name}.png")).display());
    picture
}

/// How sharp a part of a picture is: the mean change of brightness from a pixel to the next one
/// on its right, from 0 to 255. Text and edges seen through a working blur score near 0.
fn sharpness(picture: &image::RgbaImage, area: egui::Rect) -> f32 {
    let light = |x: u32, y: u32| {
        let [r, g, b, _] = picture.get_pixel(x, y).0;
        0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b)
    };
    let (mut sum, mut count) = (0.0, 0.0);
    for y in area.top() as u32..area.bottom() as u32 {
        for x in area.left() as u32..area.right() as u32 - 1 {
            sum += (light(x + 1, y) - light(x, y)).abs();
            count += 1.0;
        }
    }
    sum / count
}

/// The glass blurs what is behind it: the same picture drawn with the blur and without it, and
/// the part of the glass with nothing drawn on it is sharp without the blur and soft with it.
fn blurs(scene: &mut Scene, name: &str, areas: &[(&str, egui::Rect)]) {
    let soft = shoot(scene, name);
    scene.app.sharp_glass_for_test(true);
    let sharp = shoot(scene, &format!("{name}-sharp"));
    scene.app.sharp_glass_for_test(false);
    for (what, area) in areas {
        let (soft, sharp) = (sharpness(&soft, *area), sharpness(&sharp, *area));
        println!("{name} {what}: blurred {soft:.2}, sharp {sharp:.2}");
        assert!(sharp >= 2.5, "{name}: nothing sharp behind the {what} to blur ({sharp:.2})");
        assert!(soft <= 1.5 && soft <= sharp * 0.3, "{name}: the {what} does not blur ({soft:.2} against {sharp:.2})");
    }
}

/// The glass theme, as the sketch in `design/sketch` draws it with the same data.
#[test]
fn the_glass_theme_draws() {
    use telinha::appearance::{Appearance, ThemeChoice};
    let mut scene = scene();
    let (server_id, channel) = (scene.server_id.clone(), scene.text_channel.clone());
    scene.app.navigate(Nav::Server { server_id, channel_id: Some(channel) });
    scene.app.set_appearance(Appearance { theme: ThemeChoice::Glass, ..Appearance::default() });
    shoot(&mut scene, "glass-server");
    scene.app.heads_up_for_test(&["dm"]);
    shoot(&mut scene, "glass-heads-up");
    scene.app.ring_for_test("Keep", false);
    scene.app.heads_up_for_test(&["mention", "live"]);
    shoot(&mut scene, "glass-heads-up-stack");
    scene.app.ring_for_test("", false);
    scene.app.heads_up_for_test(&[]);
    scene.app.open_thread_for_test("m1");
    shoot(&mut scene, "glass-thread");
    scene.app.close_thread_for_test();
    scene.app.pins_for_test(true);
    shoot(&mut scene, "glass-pins");
    scene.app.pins_for_test(false);
    scene.app.open_settings();
    scene.app.profile_draft_for_test(Some(("Raphalsk", "ele/dele")));
    shoot(&mut scene, "glass-settings-profile");
    scene.app.profile_draft_for_test(None);
    scene.app.settings_page_for_test(telinha::settings::Page::Appearance);
    shoot(&mut scene, "glass-settings");
    scene.app.scroll_settings_for_test(560.0);
    shoot(&mut scene, "glass-settings-icons");
    scene.app.settings_page_for_test(telinha::settings::Page::Voice);
    shoot(&mut scene, "glass-voice-settings");
    scene.app.sensitivity_for_test(false, -45.0);
    shoot(&mut scene, "glass-voice-manual");
    scene.app.sensitivity_for_test(true, -50.0);
    scene.app.close_settings();
    let voice = scene.voice_channel.clone();
    scene.app.navigate(Nav::Server { server_id: scene.server_id.clone(), channel_id: Some(voice) });
    shoot(&mut scene, "glass-voice");
    scene.app.navigate(Nav::Home);
    shoot(&mut scene, "glass-home");
    scene.app.open_dialog(telinha::dialogs::Dialog::server("rapha"));
    // The veil of a dialog blurs the window around it: the cards of the home page at the right.
    blurs(&mut scene, "glass-dialog", &[("veil", egui::Rect::from_min_max(egui::pos2(1010.0, 215.0), egui::pos2(1260.0, 310.0)))]);
    scene.app.close_dialog();
    let (server_id, room) = (scene.server_id.clone(), scene.voice_channel.clone());
    {
        let _guard = scene.runtime.enter();
        let signaling = scene.hub.signaling();
        signaling.loopback(&server_id, &presence("peer-keep", "Keep", KEEP, json!({ "voice": { "roomId": room, "mic": true, "live": true, "liveName": "Jogo", "v": 2 } })));
        signaling.loopback(&server_id, &presence("peer-nanda", "nanda", NANDA, json!({ "voice": { "roomId": room, "mic": false, "v": 2 } })));
        scene.runtime.block_on(async { tokio::time::sleep(Duration::from_millis(100)).await });
        scene.hub.join_voice(&server_id, &room, true, false, false).unwrap();
        scene.runtime.block_on(async { tokio::time::sleep(Duration::from_millis(100)).await });
    }
    scene.app.navigate(Nav::Server { server_id: server_id.clone(), channel_id: Some(room) });
    shoot(&mut scene, "glass-room");
    // The buttons coming out of one pill and pulling apart as the call opens.
    for (index, seconds) in [0.12, 0.3, 0.55, 0.75, 0.9].into_iter().enumerate() {
        scene.app.room_opening_for_test(Some(seconds));
        shoot(&mut scene, &format!("glass-room-opening-{index}"));
    }
    scene.app.room_opening_for_test(None);
    shoot_hovering(&mut scene.app, "glass-hang-up", egui::pos2(811.0, 672.0));
    scene.app.room_chat_for_test(true);
    shoot(&mut scene, "glass-room-chat");
    scene.app.room_chat_for_test(false);
    shoot_clicking(&mut scene.app, "glass-menu", &[(egui::pos2(1190.0, 126.0), egui::PointerButton::Secondary)]);
    for name in ["Risada", "Buzina", "Tambores", "Aplausos"] {
        scene.hub.add_sound(name, "audio/mpeg", b"not really a sound").unwrap();
    }
    scene.app.open_soundboard();
    shoot(&mut scene, "glass-soundboard");
    scene.app.close_soundboard();
    {
        let _guard = scene.runtime.enter();
        scene.hub.leave_voice();
    }
    scene.app.disable_calls();
    // The conversation scrolled so that its text runs under the message box.
    let channel = scene.text_channel.clone();
    scene.app.navigate(Nav::Server { server_id, channel_id: Some(channel) });
    // Past the bar of the thread of the first message, which pushes the rest down.
    scene.app.scroll_chat_for_test(38.0);
    blurs(&mut scene, "glass-blur", &[
        // The name over the last message, under the message box and below what is written in it.
        ("message box", egui::Rect::from_min_max(egui::pos2(404.0, 806.0), egui::pos2(600.0, 817.0))),
        // The last message, under the band of the notice and below the notice itself.
        ("notice", egui::Rect::from_min_max(egui::pos2(404.0, 840.0), egui::pos2(456.0, 847.0))),
    ]);
    // The tools over a message, and the bar of its thread.
    scene.app.scroll_chat_for_test(0.0);
    shoot_hovering(&mut scene.app, "glass-message-tools", egui::pos2(700.0, 300.0));
    shoot_hovering(&mut scene.app, "glass-thread-bar", egui::pos2(470.0, 337.0));
}

#[test]
fn the_window_draws() {
    let mut scene = scene();
    // The pictures of this test are of the gray theme; the glass has a test of its own.
    let gray = telinha::appearance::Appearance { theme: telinha::appearance::ThemeChoice::Ash, ..telinha::appearance::Appearance::default() };
    scene.app.set_appearance(gray);
    let (server_id, channel) = (scene.server_id.clone(), scene.text_channel.clone());
    scene.app.navigate(Nav::Server { server_id, channel_id: Some(channel) });
    shoot(&mut scene, "server");
    scene.app.open_thread_for_test("m1");
    shoot(&mut scene, "thread");
    scene.app.close_thread_for_test();
    scene.app.pins_for_test(true);
    shoot(&mut scene, "pins");
    scene.app.pins_for_test(false);
    shoot_clicking(&mut scene.app, "menu-server", &[(egui::pos2(190.0, 24.0), egui::PointerButton::Primary)]);
    shoot_clicking(&mut scene.app, "menu-person", &[(egui::pos2(1190.0, 113.0), egui::PointerButton::Secondary)]);
    shoot_clicking(&mut scene.app, "menu-user", &[(egui::pos2(150.0, 832.0), egui::PointerButton::Secondary)]);
    scene.app.open_card(Some(KEEP.to_owned()), "Keep", egui::Rect::from_min_size(egui::pos2(1120.0, 60.0), egui::vec2(232.0, 42.0)));
    shoot(&mut scene, "profile-card");
    scene.app.close_card();
    scene.app.open_dialog(telinha::dialogs::Dialog::server("rapha"));
    shoot(&mut scene, "dialog-add-server");
    let code = scene.hub.invite_code(&scene.server_id).unwrap();
    scene.app.open_dialog(telinha::dialogs::Dialog::Invite { server_name: "Servidor de rapha".into(), code, feedback: String::new(), sent: Vec::new() });
    shoot(&mut scene, "dialog-invite");
    scene.app.open_dialog(telinha::dialogs::Dialog::channel(&scene.server_id, None, ChannelKind::Text));
    shoot(&mut scene, "dialog-channel");
    scene.app.open_dialog(telinha::dialogs::Dialog::for_purpose(telinha::dialogs::Purpose::Contact(KEEP.to_owned()), "Keep"));
    shoot(&mut scene, "dialog-prompt");
    scene.app.close_dialog();
    {
        let _guard = scene.runtime.enter();
        // Three avatars one after the other: the last one is in use, all three are recent.
        for hue in [10, 160, 90] {
            let avatar = telinha::settings::prepare_picture(&png(600, 400, hue), (256, 256)).unwrap();
            assert!(scene.hub.set_picture(false, "image/webp", avatar));
        }
        assert_eq!(scene.hub.recent_avatars().len(), 3);
        scene.hub.set_style(&json!({ "name": { "effect": "prism", "colors": ["#19b8b0", "#f0b232", "#eb459e"] }, "theme": ["#19b8b0", "#5865f2"], "bio": "Faço o Telinha." }));
        assert_eq!(scene.hub.profile_style().name.unwrap().colors.len(), 3);
    }
    scene.app.open_settings();
    shoot(&mut scene, "settings");
    // A change not saved brings up the bar that saves or undoes it.
    scene.app.profile_draft_for_test(Some(("Rapha", "ele/dele")));
    shoot(&mut scene, "settings-unsaved");
    scene.app.scroll_settings_for_test(420.0);
    shoot(&mut scene, "settings-profile-2");
    scene.app.profile_draft_for_test(None);
    scene.app.open_avatar_picker();
    shoot(&mut scene, "avatar-picker");
    scene.app.close_avatar_picker();
    scene.app.open_crop(&png(900, 500, 30), false);
    shoot(&mut scene, "crop-avatar");
    scene.app.open_crop(&png(900, 500, 30), true);
    shoot(&mut scene, "crop-banner");
    scene.app.close_crop();
    {
        use telinha::appearance::{Appearance, Density, MessageStyle, ThemeChoice};
        let plain = Appearance { theme: ThemeChoice::Ash, ..Appearance::default() };
        use telinha::settings::Page;
        scene.app.settings_page_for_test(Page::Voice);
        shoot(&mut scene, "settings-voice");
        scene.app.settings_page_for_test(Page::Appearance);
        shoot(&mut scene, "settings-appearance");
        scene.app.scroll_settings_for_test(400.0);
        shoot(&mut scene, "settings-appearance-2");
        scene.app.settings_page_for_test(Page::Messages);
        shoot(&mut scene, "settings-messages");
        {
            use telinha::shortcuts::{Action, Shortcut};
            scene.app.settings_page_for_test(Page::Shortcuts);
            shoot(&mut scene, "settings-shortcuts-empty");
            let list = vec![
                Shortcut { action: Action::ToggleMute, keys: vec![0x11, 0x10, 0x4D], enabled: true },
                Shortcut { action: Action::PushToTalk, keys: vec![0x05], enabled: true },
                Shortcut { action: Action::ToggleDeaf, keys: vec![0x12, 0x44], enabled: false },
                Shortcut::new(Action::ShowWindow),
            ];
            scene.app.shortcuts_for_test(list.clone(), Some((3, Vec::new())));
            shoot(&mut scene, "settings-shortcuts");
            shoot_hovering(&mut scene.app, "settings-shortcuts-hover", egui::pos2(800.0, 300.0));
            scene.app.shortcuts_for_test(list, Some((2, vec![0x11, 0x44])));
            scene.app.scroll_settings_for_test(300.0);
            shoot(&mut scene, "settings-shortcuts-2");
            scene.app.shortcuts_for_test(Vec::new(), None);
        }
        scene.app.update_info_for_test(true, "Você já tem a versão mais nova (0.4.0). Conferido agora há pouco.");
        scene.app.settings_page_for_test(Page::Updates);
        shoot(&mut scene, "settings-updates");
        scene.app.close_settings();
        scene.app.set_appearance(Appearance { theme: ThemeChoice::Light, ..plain.clone() });
        shoot(&mut scene, "server-light");
        scene.app.set_appearance(Appearance { theme: ThemeChoice::Light, dark_sidebar: true, ..plain.clone() });
        shoot(&mut scene, "server-light-dark-sidebar");
        scene.app.set_appearance(Appearance { theme: ThemeChoice::Color(0), ..plain.clone() });
        shoot(&mut scene, "server-aurora");
        scene.app.set_appearance(Appearance { theme: ThemeChoice::Color(9), density: Density::Compact, messages: MessageStyle::Compact, chat_font: 13.0, message_gap: 4.0, ..plain.clone() });
        shoot(&mut scene, "server-citrus-compact");
        scene.app.set_appearance(plain);
    }
    {
        // Messages for conversations that are not open leave a count, and a call rings.
        let _guard = scene.runtime.enter();
        let signaling = scene.hub.signaling();
        let other_channel = scene.hub.server(&scene.server_id).unwrap().channels.iter().find(|channel| channel.name == "canal-dos-memes").unwrap().id.clone();
        for index in 0..3 {
            signaling.loopback(&scene.server_id, &json!({
                "type": "chat", "from": "peer-keep", "at": now_ms(), "mid": format!("mid-unread-{index}"),
                "messageId": format!("unread-{index}"), "channelId": other_channel, "author": "Keep", "memberId": KEEP,
                "sentAt": now_ms(), "text": "olha isso",
            }));
        }
        scene.runtime.block_on(async { tokio::time::sleep(Duration::from_millis(100)).await });
    }
    shoot(&mut scene, "unread");
    scene.app.link_for_test("https://example.com/receita", "example.com", "Receita de bolo de cenoura", "Fofinho, com cobertura de chocolate, pronto em 40 minutos.");
    shoot(&mut scene, "link-draft");
    let pasted: Vec<String> = (1..=60).map(|line| format!("linha {line} de um texto colado bem comprido, que passa do limite de uma mensagem")).collect();
    scene.app.draft_for_test(&pasted.join("
"));
    shoot(&mut scene, "long-draft");
    let long = scene.app.composer_height_for_test();
    scene.app.draft_for_test("");
    shoot(&mut scene, "draft-cleared");
    let cleared = scene.app.composer_height_for_test();
    println!("message box: {long} with the long draft, {cleared} after clearing it");
    assert!(cleared < long - 100.0, "the message box did not shrink back ({long} to {cleared})");
    assert_eq!(scene.app.unread(&telinha_core::hub::Conversation::channel(&scene.server_id, &scene.text_channel)), 0);
    scene.app.ring_for_test("Keep", true);
    shoot(&mut scene, "incoming-call");
    scene.app.ring_for_test("", false);
    // The cards at the top right: a direct message alone, then a call over a mention and a share.
    scene.app.heads_up_for_test(&["dm"]);
    shoot(&mut scene, "heads-up");
    scene.app.ring_for_test("Keep", false);
    scene.app.heads_up_for_test(&["mention", "live"]);
    shoot(&mut scene, "heads-up-stack");
    scene.app.ring_for_test("", false);
    scene.app.heads_up_for_test(&[]);
    scene.app.open_picture("bytes://test/alta.png", png(700, 1900, 60));
    shoot(&mut scene, "lightbox-tall");
    scene.app.open_picture("bytes://test/larga.png", png(2000, 620, 140));
    shoot(&mut scene, "lightbox-wide");
    scene.app.close_picture();
    scene.app.disable_calls();
    let (server_id, room) = (scene.server_id.clone(), scene.voice_channel.clone());
    {
        let _guard = scene.runtime.enter();
        let signaling = scene.hub.signaling();
        signaling.loopback(&server_id, &presence("peer-keep", "Keep", KEEP, json!({ "voice": { "roomId": room, "mic": true, "live": true, "liveName": "Jogo", "v": 2 } })));
        signaling.loopback(&server_id, &presence("peer-nanda", "nanda", NANDA, json!({ "voice": { "roomId": room, "mic": false, "v": 2 } })));
        scene.runtime.block_on(async { tokio::time::sleep(Duration::from_millis(100)).await });
    }
    scene.app.navigate(Nav::Server { server_id: server_id.clone(), channel_id: Some(room.clone()) });
    shoot(&mut scene, "lobby");
    {
        let _guard = scene.runtime.enter();
        scene.hub.join_voice(&server_id, &room, true, false, false).unwrap();
        scene.runtime.block_on(async { tokio::time::sleep(Duration::from_millis(100)).await });
    }
    shoot(&mut scene, "room");
    {
        // A picture with a white square at the top left: in the mirror it shows at the right.
        let (width, height) = (640u32, 360u32);
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let pixel = if x < 120 && y < 120 { [255, 255, 255, 255] } else if x < width / 2 { [220, 120, 40, 255] } else { [40, 90, 200, 255] };
                rgba.extend_from_slice(&pixel);
            }
        }
        scene.app.camera_for_test(width, height, rgba);
        shoot(&mut scene, "room-camera");
        scene.app.set_camera(false);
        // A screen of Keep watched inside the window: a dark desktop with a bright window on it.
        let (width, height) = (1280u32, 720u32);
        let mut screen = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let inside = (200..900).contains(&x) && (120..560).contains(&y);
                let bar = inside && y < 160;
                let pixel = if bar { [60, 64, 72, 255] } else if inside { [235, 236, 240, 255] } else { [20, (40 + y / 12) as u8, (80 + x / 10) as u8, 255] };
                screen.extend_from_slice(&pixel);
            }
        }
        scene.app.stream_for_test("peer-keep", "Keep", width, height, screen);
        shoot(&mut scene, "room-stream");
        scene.app.open_viewer("peer-keep", "Tela de Keep".to_owned());
        shoot(&mut scene, "stream-viewer");
        shoot_clicking(&mut scene.app, "stream-viewer-menu", &[(egui::pos2(600.0, 400.0), egui::PointerButton::Secondary)]);
        scene.app.close_viewer();
    }
    scene.app.focus_tile(Some("peer-keep"));
    shoot(&mut scene, "room-focus");
    scene.app.focus_tile(None);
    scene.app.toggle_voice_popover();
    shoot(&mut scene, "voice-popover");
    scene.app.toggle_voice_popover();
    {
        // Any bytes do for the picture of the soundboard: nothing is played here.
        for name in ["Risada", "Buzina", "Tambores", "Aplausos"] {
            scene.hub.add_sound(name, "audio/mpeg", b"not really a sound").unwrap();
        }
        scene.app.open_soundboard();
        shoot(&mut scene, "soundboard");
        scene.app.close_soundboard();
        for step in 0..40u32 {
            let wave = f64::from(step) * 0.4;
            scene.app.debug_for_test(&json!({
                "event": "stats", "role": "sender", "frames": step * 60, "width": 2560, "height": 1440, "rtt_ms": 38.0 + wave.sin() * 14.0,
                "loss": 0.004 + (wave.cos().abs()) * 0.01, "bitrate_bps": 7_800_000.0, "capture_ms": 1.4, "encode_ms": 3.1, "keyframes": 3, "viewers": 2,
            }));
        }
        scene.app.open_debug();
        shoot(&mut scene, "debug");
        scene.app.close_debug();
    }
    {
        use telinha::stream_settings::{AudioChoice, Preset, Quality, RateMode, Shared};
        let quality = Quality { preset: Preset::Custom, resolution: 2, fps: 3, bitrate: 14, typed_kbps: 25_000, mode: RateMode::Variable, ..Quality::default() };
        scene.app.stream_settings_for_test(Some(Shared { window_pid: 0, audio: AudioChoice::Computer, quality, manual: false, ..Shared::default() }));
        shoot(&mut scene, "stream-settings");
        let fixed = Quality { mode: RateMode::Fixed, image: 85, bitrate: 4, ..quality };
        scene.app.stream_settings_for_test(Some(Shared { quality: fixed, ..Shared::default() }));
        shoot(&mut scene, "stream-settings-fixed");
        scene.app.stream_settings_for_test(None);
    }
    {
        let targets = telinha_core::engine::Targets {
            monitors: vec![
                json!({ "handle": 1, "name": "AW2525HM", "width": 1920, "height": 1080, "refresh_hz": 239.76, "primary": true }),
                json!({ "handle": 2, "name": "ZOWIE XL LCD", "width": 1920, "height": 1080, "refresh_hz": 144.0 }),
            ],
            windows: vec![json!({ "handle": 3, "name": "Steam", "width": 1920, "height": 1032, "pid": 40 })],
            devices: Vec::new(),
            audio_inputs: Vec::new(),
        };
        scene.app.picker_for_test(false, targets.clone());
        shoot(&mut scene, "share-picker");
        scene.app.picker_custom_for_test();
        shoot(&mut scene, "share-picker-custom");
        scene.app.picker_for_test(true, targets);
        shoot(&mut scene, "share-picker-switch");
        scene.app.close_picker_for_test();
    }
    scene.app.navigate(Nav::Home);
    shoot(&mut scene, "home");
    scene.app.manual_for_test(&[
        json!({ "event": "step", "text": "preparando a captura" }),
        json!({ "event": "code", "kind": "invite", "code": format!("TELINHA1.{}", "Qm9sbyBkZSBjZW5vdXJhIGNvbSBjb2JlcnR1cmE".repeat(9)) }),
        json!({ "event": "need_code", "kind": "answer" }),
    ]);
    shoot(&mut scene, "home-codes");
    scene.app.manual_for_test(&[json!({ "event": "state", "state": "Connected" })]);
    shoot(&mut scene, "home-live");
    {
        use telinha_core::updater::{UpdateState, UpdateStatus};
        scene.app.show_update_for_test(Some(UpdateState { status: UpdateStatus::Ready, version: Some("0.4.0".into()), progress: 1.0 }));
        shoot(&mut scene, "update-ready");
        scene.app.show_update_for_test(None);
    }
    assert_eq!(scene.hub.servers().len(), 2);
    drop(scene.app);
    scene.runtime.shutdown_background();
}

/// Run by hand: `cargo test -p telinha --test snapshots real_profile -- --ignored --nocapture`.
/// Draws the window over a copy of the data the Electron app left on this machine, without
/// connecting anywhere. The picture stays in `client/target/snapshots` and holds private chat.
#[test]
#[ignore]
fn a_copy_of_the_real_profile_draws() {
    let source = PathBuf::from(std::env::var("APPDATA").unwrap()).join("Telinha");
    let folder = tempfile::tempdir().unwrap();
    for name in ["Local State", "profile.json", "contacts.json", "servers.json"] {
        let _ = std::fs::copy(source.join(name), folder.path().join(name));
    }
    for name in ["chats", "avatars", "banners"] {
        copy_folder(&source.join(name), &folder.path().join(name));
    }
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let guard = runtime.enter();
    // No message server: the copy never announces itself to anyone.
    let (hub, events) = Hub::open(folder.path(), Vec::new());
    drop(guard);
    let servers = hub.servers();
    println!("{} servers, {} contacts, name {}", servers.len(), hub.contacts().len(), hub.profile_name());
    for server in &servers {
        for channel in &server.channels {
            let messages = hub.history(&telinha_core::hub::Conversation::channel(&server.id, &channel.id));
            let pictures = messages.iter().filter(|message| message.image.is_some()).count();
            println!("  {} / {}: {} messages, {} with a picture", server.name, channel.name, messages.len(), pictures);
        }
    }
    let context = egui::Context::default();
    let mut app = TelinhaApp::new(&context, Arc::clone(&hub), events, runtime.handle().clone());
    app.disable_calls();
    shoot_app(&mut app, "real-profile");
    assert!(!servers.is_empty());
    drop(app);
    runtime.shutdown_background();
}

fn copy_folder(from: &std::path::Path, to: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(from) else { return };
    std::fs::create_dir_all(to).unwrap();
    for entry in entries.flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_folder(&entry.path(), &target);
        } else {
            let _ = std::fs::copy(entry.path(), target);
        }
    }
}