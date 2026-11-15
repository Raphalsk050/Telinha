//! The icons of the interface, drawn in white so each use tints them. Generated from the
//! drawings of the old interface; edit the files in `assets/icons`, not this list.

use egui::{Color32, Image, Vec2};

pub const ATTACH: Icon = Icon("attach", include_bytes!("../assets/icons/attach.svg"));
pub const CAMERA: Icon = Icon("camera", include_bytes!("../assets/icons/camera.svg"));
pub const CHEVRON_DOWN: Icon = Icon("chevron_down", include_bytes!("../assets/icons/chevron_down.svg"));
pub const CLOSE: Icon = Icon("close", include_bytes!("../assets/icons/close.svg"));
pub const DEAF: Icon = Icon("deaf", include_bytes!("../assets/icons/deaf.svg"));
pub const DELETE: Icon = Icon("delete", include_bytes!("../assets/icons/delete.svg"));
pub const DOWNLOAD: Icon = Icon("download", include_bytes!("../assets/icons/download.svg"));
pub const EXPAND: Icon = Icon("expand", include_bytes!("../assets/icons/expand.svg"));
pub const FILE: Icon = Icon("file", include_bytes!("../assets/icons/file.svg"));
pub const GEAR: Icon = Icon("gear", include_bytes!("../assets/icons/gear.svg"));
pub const HANGUP: Icon = Icon("hangup", include_bytes!("../assets/icons/hangup.svg"));
pub const HASH: Icon = Icon("hash", include_bytes!("../assets/icons/hash.svg"));
pub const HEADPHONES: Icon = Icon("headphones", include_bytes!("../assets/icons/headphones.svg"));
pub const HOME: Icon = Icon("home", include_bytes!("../assets/icons/home.svg"));
pub const INVITE: Icon = Icon("invite", include_bytes!("../assets/icons/invite.svg"));
pub const LOCK: Icon = Icon("lock", include_bytes!("../assets/icons/lock.svg"));
pub const MEMBERS: Icon = Icon("members", include_bytes!("../assets/icons/members.svg"));
pub const MIC: Icon = Icon("mic", include_bytes!("../assets/icons/mic.svg"));
pub const MIC_OFF: Icon = Icon("mic_off", include_bytes!("../assets/icons/mic_off.svg"));
pub const MINUS: Icon = Icon("minus", include_bytes!("../assets/icons/minus.svg"));
pub const MORE: Icon = Icon("more", include_bytes!("../assets/icons/more.svg"));
pub const PHONE: Icon = Icon("phone", include_bytes!("../assets/icons/phone.svg"));
pub const PLUS: Icon = Icon("plus", include_bytes!("../assets/icons/plus.svg"));
pub const POPOUT: Icon = Icon("popout", include_bytes!("../assets/icons/popout.svg"));
pub const PULSE: Icon = Icon("pulse", include_bytes!("../assets/icons/pulse.svg"));
pub const ROTATE: Icon = Icon("rotate", include_bytes!("../assets/icons/rotate.svg"));
pub const SCREEN: Icon = Icon("screen", include_bytes!("../assets/icons/screen.svg"));
pub const SEND: Icon = Icon("send", include_bytes!("../assets/icons/send.svg"));
pub const SERVER_BIG: Icon = Icon("server_big", include_bytes!("../assets/icons/server_big.svg"));
pub const SHARE_BIG: Icon = Icon("share_big", include_bytes!("../assets/icons/share_big.svg"));
pub const SIGNAL: Icon = Icon("signal", include_bytes!("../assets/icons/signal.svg"));
pub const SLIDERS: Icon = Icon("sliders", include_bytes!("../assets/icons/sliders.svg"));
pub const SOUNDBOARD: Icon = Icon("soundboard", include_bytes!("../assets/icons/soundboard.svg"));
pub const SPEAKER: Icon = Icon("speaker", include_bytes!("../assets/icons/speaker.svg"));
pub const SWITCH_SCREEN: Icon = Icon("switch_screen", include_bytes!("../assets/icons/switch_screen.svg"));
pub const UPLOAD_PICTURE: Icon = Icon("upload_picture", include_bytes!("../assets/icons/upload_picture.svg"));
pub const WATCH_BIG: Icon = Icon("watch_big", include_bytes!("../assets/icons/watch_big.svg"));
pub const TARGET_MONITOR: Icon = Icon("target_monitor", include_bytes!("../assets/icons/target_monitor.svg"));
pub const TARGET_WINDOW: Icon = Icon("target_window", include_bytes!("../assets/icons/target_window.svg"));
pub const TARGET_DEVICE: Icon = Icon("target_device", include_bytes!("../assets/icons/target_device.svg"));

#[derive(Clone, Copy)]
pub struct Icon(&'static str, &'static [u8]);

impl Icon {
    /// The icon as a widget of the given side, in the given color.
    pub fn image(self, side: f32, color: Color32) -> Image<'static> {
        Image::from_bytes(format!("bytes://icons/{}.svg", self.0), self.1).fit_to_exact_size(Vec2::splat(side)).tint(color)
    }
}
