//! The notices Windows shows in the corner of the screen when a message or a call arrives and
//! the person is not looking.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use telinha_core::avatars::Picture;
use telinha_core::hub::Conversation;

/// The picture of the app, for the title bar and for the notices.
pub const ICON: &[u8] = include_bytes!("../../../desktop/assets/icon.png");

/// The name Windows knows the app by. The same one as before, so the notices of both versions
/// of the app fall under one entry of the Windows settings.
pub const APP_ID: &str = "dev.telinha.desktop";

/// How many notices are kept alive. Windows forgets the click of a notice whose object is gone.
const KEPT: usize = 12;

pub struct Notices {
    #[cfg(windows)]
    notifier: Option<windows::UI::Notifications::ToastNotifier>,
    #[cfg(windows)]
    shown: VecDeque<windows::UI::Notifications::ToastNotification>,
    pictures: PathBuf,
    clicked: Sender<Conversation>,
    clicks: Receiver<Conversation>,
    wake: egui::Context,
}

fn escape(text: &str) -> String {
    text.chars()
        .filter(|letter| !letter.is_control() || *letter == '\n')
        .map(|letter| match letter {
            '&' => "&amp;".to_owned(),
            '<' => "&lt;".to_owned(),
            '>' => "&gt;".to_owned(),
            '"' => "&quot;".to_owned(),
            '\'' => "&apos;".to_owned(),
            other => other.to_string(),
        })
        .collect()
}

/// What Windows is asked to show: two lines of text and, when there is one, the face of who
/// sent it in a circle.
pub fn toast_xml(title: &str, body: &str, picture: Option<&Path>) -> String {
    let image = picture.map_or_else(String::new, |file| {
        let address = file.to_string_lossy().replace('\\', "/");
        format!(r#"<image placement="appLogoOverride" hint-crop="circle" src="file:///{}"/>"#, escape(&address))
    });
    format!(
        r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text>{image}</binding></visual></toast>"#,
        escape(title),
        escape(body)
    )
}

impl Notices {
    /// Tells Windows who the app is, which is what lets its notices appear at all.
    pub fn new(data_dir: &Path, wake: &egui::Context) -> Self {
        let (clicked, clicks) = channel();
        let pictures = data_dir.join("notice-pictures");
        #[cfg(windows)]
        let notifier = register();
        Self {
            #[cfg(windows)]
            notifier,
            #[cfg(windows)]
            shown: VecDeque::new(),
            pictures,
            clicked,
            clicks,
            wake: wake.clone(),
        }
    }

    /// The conversation of a notice the person clicked, when there is one.
    pub fn clicked(&self) -> Option<Conversation> {
        self.clicks.try_recv().ok()
    }

    /// Windows shows PNG and JPEG in a notice but not WebP, which is what most pictures are.
    fn picture_file(&self, picture: &Picture) -> Option<PathBuf> {
        let file = self.pictures.join(format!("{}.png", picture.hash));
        if !file.is_file() {
            let decoded = image::load_from_memory(&picture.data).ok()?.thumbnail(128, 128);
            std::fs::create_dir_all(&self.pictures).ok()?;
            // Only the faces of the latest notices are worth the room they take.
            if let Ok(files) = std::fs::read_dir(&self.pictures) {
                let mut files: Vec<_> = files.flatten().filter_map(|file| Some((file.metadata().ok()?.modified().ok()?, file.path()))).collect();
                files.sort();
                for (_, old) in files.iter().rev().skip(40) {
                    let _ = std::fs::remove_file(old);
                }
            }
            decoded.save_with_format(&file, image::ImageFormat::Png).ok()?;
        }
        Some(file)
    }

    #[cfg(windows)]
    pub fn show(&mut self, title: &str, body: &str, picture: Option<&Picture>, open: Conversation) -> bool {
        use windows::Data::Xml::Dom::XmlDocument;
        use windows::Foundation::TypedEventHandler;
        use windows::UI::Notifications::ToastNotification;
        use windows::core::HSTRING;

        let Some(notifier) = self.notifier.clone() else { return false };
        let file = picture.and_then(|picture| self.picture_file(picture));
        let xml = toast_xml(title, body, file.as_deref());
        let clicked = self.clicked.clone();
        let wake = self.wake.clone();
        let shown = (|| -> windows::core::Result<ToastNotification> {
            let document = XmlDocument::new()?;
            document.LoadXml(&HSTRING::from(xml))?;
            let toast = ToastNotification::CreateToastNotification(&document)?;
            toast.Activated(&TypedEventHandler::new(move |_, _| {
                let _ = clicked.send(open.clone());
                wake.request_repaint();
                Ok(())
            }))?;
            notifier.Show(&toast)?;
            Ok(toast)
        })();
        let Ok(toast) = shown else { return false };
        self.shown.push_back(toast);
        while self.shown.len() > KEPT {
            self.shown.pop_front();
        }
        true
    }

    #[cfg(not(windows))]
    pub fn show(&mut self, _title: &str, _body: &str, _picture: Option<&Picture>, _open: Conversation) -> bool {
        false
    }
}

/// Writes the name and the picture of the app where Windows looks them up for a program that
/// was not installed, and gets the object that shows notices.
#[cfg(windows)]
fn register() -> Option<windows::UI::Notifications::ToastNotifier> {
    use windows::UI::Notifications::ToastNotificationManager;
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    use windows::core::HSTRING;

    let key = HSTRING::from(format!("Software\\Classes\\AppUserModelId\\{APP_ID}"));
    let set = |name: &str, value: &str| {
        let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: the strings are owned and end in zero, and the size is that of the buffer.
        unsafe {
            let _ = RegSetKeyValueW(HKEY_CURRENT_USER, &key, &HSTRING::from(name), REG_SZ.0, Some(wide.as_ptr().cast()), (wide.len() * 2) as u32);
        }
    };
    set("DisplayName", "Telinha");
    if let Some(folder) = std::env::var_os("LOCALAPPDATA").map(|base| PathBuf::from(base).join("Telinha")) {
        let icon = folder.join("icon.png");
        let whole = std::fs::metadata(&icon).is_ok_and(|metadata| metadata.len() == ICON.len() as u64);
        if whole || (std::fs::create_dir_all(&folder).is_ok() && std::fs::write(&icon, ICON).is_ok()) {
            set("IconUri", &icon.to_string_lossy());
        }
    }
    // SAFETY: a plain call with an owned string.
    unsafe {
        let _ = SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(APP_ID));
    }
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_people_write_cannot_break_the_notice() {
        let xml = toast_xml("Keep em Servidor #geral", "olha <b>isso</b> & \"aquilo\"\u{7}", None);
        assert_eq!(
            xml,
            r#"<toast><visual><binding template="ToastGeneric"><text>Keep em Servidor #geral</text><text>olha &lt;b&gt;isso&lt;/b&gt; &amp; &quot;aquilo&quot;</text></binding></visual></toast>"#
        );
        let with_face = toast_xml("a", "b", Some(Path::new(r"C:\Users\x\AppData\Roaming\Telinha\notice-pictures\ab.png")));
        assert!(with_face.contains(r#"src="file:///C:/Users/x/AppData/Roaming/Telinha/notice-pictures/ab.png""#));
    }

    /// Run by hand: `cargo test -p telinha --lib notify -- --ignored`. Shows one real notice on
    /// this computer, with a face, to check that Windows takes it.
    #[test]
    #[ignore]
    fn windows_takes_a_notice() {
        let folder = tempfile::tempdir().unwrap();
        let mut notices = Notices::new(folder.path(), &egui::Context::default());
        let face = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(64, 64, image::Rgb([88, 101, 242])));
        let mut data = Vec::new();
        face.write_to(&mut std::io::Cursor::new(&mut data), image::ImageFormat::Png).unwrap();
        let picture = Picture { mime: "image/png".into(), hash: "teste".into(), data };
        assert!(notices.show("Telinha", "Aviso de teste do app novo.", Some(&picture), Conversation::direct("ninguem")));
        std::thread::sleep(std::time::Duration::from_secs(3));
        assert!(notices.clicked().is_none());
    }
}
