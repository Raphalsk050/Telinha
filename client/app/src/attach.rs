//! What goes with a message besides text: a picture, shrunk until it fits in a message, or a
//! file. Also the clipboard and the save dialog.

use std::path::Path;

use image::{DynamicImage, GenericImageView};
use telinha_core::avatars::{is_gif, is_jpeg, is_png, is_webp};
use telinha_core::hub::{OutgoingFile, OutgoingImage};
use telinha_core::transfers::{MAX_FILE_BYTES, MAX_RELAY_BYTES};

/// The largest picture that travels inside a message.
pub const MAX_IMAGE_BYTES: usize = 380 * 1024;
/// A GIF above the picture limit travels as a file and is shown animated up to this size.
pub const MAX_INLINE_GIF_BYTES: i64 = 24 * 1024 * 1024;
const IMAGE_SIDES: [u32; 5] = [1920, 1600, 1280, 960, 720];
const IMAGE_QUALITIES: [f32; 3] = [85.0, 72.0, 60.0];

pub enum Attachment {
    Image { image: OutgoingImage, name: String },
    File(OutgoingFile),
}

impl Attachment {
    pub fn name(&self) -> &str {
        match self {
            Self::Image { name, .. } => name,
            Self::File(file) => &file.name,
        }
    }

    pub fn size(&self) -> usize {
        match self {
            Self::Image { image, .. } => image.data.len(),
            Self::File(file) => file.size() as usize,
        }
    }
}

/// Above this size a file is not read to see whether it is a picture: it goes as a file, read
/// from the disk as it is sent.
const MAX_READ_BYTES: u64 = 64 * 1024 * 1024;

fn sniff(data: &[u8]) -> Option<&'static str> {
    if is_png(data) {
        Some("image/png")
    } else if is_jpeg(data) {
        Some("image/jpeg")
    } else if is_gif(data) {
        Some("image/gif")
    } else if is_webp(data) {
        Some("image/webp")
    } else {
        None
    }
}

fn mime_of(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// A picture as WebP small enough for a message: first smaller in quality, then in size.
pub fn encode_image(picture: &DynamicImage) -> Result<OutgoingImage, String> {
    for side in IMAGE_SIDES {
        let (width, height) = picture.dimensions();
        let scale = (side as f32 / width.max(height) as f32).min(1.0);
        let (width, height) = (((width as f32 * scale).round() as u32).max(1), ((height as f32 * scale).round() as u32).max(1));
        let resized = if scale < 1.0 { picture.resize_exact(width, height, image::imageops::FilterType::Triangle) } else { picture.clone() };
        let rgba = resized.to_rgba8();
        let encoder = webp::Encoder::from_rgba(rgba.as_raw(), width, height);
        for quality in IMAGE_QUALITIES {
            let encoded = encoder.encode(quality);
            if encoded.len() <= MAX_IMAGE_BYTES {
                return Ok(OutgoingImage { mime: "image/webp".into(), width: i64::from(width), height: i64::from(height), data: encoded.to_vec() });
            }
        }
    }
    Err("imagem grande demais".into())
}

/// The picture of a link preview: smaller and lighter than a picture sent on purpose.
pub fn encode_embed_image(picture: &DynamicImage) -> Option<OutgoingImage> {
    const SIDES: [u32; 3] = [640, 480, 360];
    const MAX_BYTES: usize = 120 * 1024;
    for side in SIDES {
        let (width, height) = picture.dimensions();
        let scale = (side as f32 / width.max(height) as f32).min(1.0);
        let (width, height) = (((width as f32 * scale).round() as u32).max(1), ((height as f32 * scale).round() as u32).max(1));
        let rgba = if scale < 1.0 { picture.resize_exact(width, height, image::imageops::FilterType::Triangle).to_rgba8() } else { picture.to_rgba8() };
        let encoder = webp::Encoder::from_rgba(rgba.as_raw(), width, height);
        for quality in IMAGE_QUALITIES {
            let encoded = encoder.encode(quality);
            if encoded.len() <= MAX_BYTES {
                return Some(OutgoingImage { mime: "image/webp".into(), width: i64::from(width), height: i64::from(height), data: encoded.to_vec() });
            }
        }
    }
    None
}

/// Turns bytes into what will go with the message. A small GIF goes as it is, to stay animated,
/// and a large one goes as a file for the same reason.
pub fn from_bytes(name: &str, data: Vec<u8>) -> Result<Attachment, String> {
    if data.is_empty() {
        return Err("o arquivo está vazio".into());
    }
    let kind = sniff(&data);
    if kind == Some("image/gif") {
        if data.len() <= MAX_IMAGE_BYTES {
            let (width, height) = image::ImageReader::new(std::io::Cursor::new(&data))
                .with_guessed_format()
                .ok()
                .and_then(|reader| reader.into_dimensions().ok())
                .ok_or("não consegui ler esse GIF")?;
            let image = OutgoingImage { mime: "image/gif".into(), width: i64::from(width), height: i64::from(height), data };
            return Ok(Attachment::Image { image, name: name.to_owned() });
        }
    } else if kind.is_some() {
        if let Ok(picture) = image::load_from_memory(&data) {
            return Ok(Attachment::Image { image: encode_image(&picture)?, name: name.to_owned() });
        }
    }
    let mime = kind.unwrap_or_else(|| mime_of(name));
    Ok(Attachment::File(OutgoingFile { name: name.to_owned(), mime: mime.to_owned(), data, path: None }))
}

pub fn from_path(path: &Path) -> Result<Attachment, String> {
    let name = path.file_name().map_or_else(|| "arquivo".to_owned(), |name| name.to_string_lossy().into_owned());
    let size = std::fs::metadata(path).map_err(|_| "não consegui ler esse arquivo")?.len();
    if size == 0 {
        return Err("o arquivo está vazio".into());
    }
    if size > MAX_FILE_BYTES {
        return Err("o arquivo precisa ter no máximo 1 GB".into());
    }
    if size > MAX_READ_BYTES {
        return Ok(Attachment::File(OutgoingFile { mime: mime_of(&name).to_owned(), name, data: Vec::new(), path: Some(path.to_owned()) }));
    }
    let attachment = from_bytes(&name, std::fs::read(path).map_err(|_| "não consegui ler esse arquivo")?)?;
    // A file above what the message servers carry is sent from where it is on the disk, not
    // from a copy in memory.
    Ok(match attachment {
        Attachment::File(file) if size > MAX_RELAY_BYTES as u64 => Attachment::File(OutgoingFile { data: Vec::new(), path: Some(path.to_owned()), ..file }),
        other => other,
    })
}

/// Asks where to save a file, starting in the downloads folder. None when the person gave up.
pub fn pick_save_path(suggested: &str) -> Option<std::path::PathBuf> {
    let downloads = std::env::var_os("USERPROFILE").map(|home| std::path::PathBuf::from(home).join("Downloads"));
    let mut dialog = rfd::FileDialog::new().set_file_name(suggested);
    if let Some(folder) = downloads.filter(|folder| folder.is_dir()) {
        dialog = dialog.set_directory(folder);
    }
    dialog.save_file()
}

/// What is on the clipboard, when it is a picture or a file.
pub fn from_clipboard() -> Option<Result<Attachment, String>> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    if let Ok(files) = clipboard.get().file_list() {
        if let Some(path) = files.first() {
            return Some(from_path(path));
        }
    }
    let picture = clipboard.get_image().ok()?;
    let rgba = image::RgbaImage::from_raw(picture.width as u32, picture.height as u32, picture.bytes.into_owned())?;
    Some(encode_image(&DynamicImage::ImageRgba8(rgba)).map(|image| Attachment::Image { image, name: "Imagem colada".to_owned() }))
}

/// Puts a picture on the clipboard as pixels, which is what other programs paste.
pub fn copy_image(bytes: &[u8]) -> Result<(), String> {
    let picture = image::load_from_memory(bytes).map_err(|_| "não consegui ler essa imagem")?.to_rgba8();
    let (width, height) = picture.dimensions();
    let data = arboard::ImageData { width: width as usize, height: height as usize, bytes: picture.into_raw().into() };
    arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_image(data)).map_err(|error| error.to_string())
}

/// Asks where to save and writes the bytes there. `Ok(false)` when the person gave up.
pub fn save_as(suggested: &str, bytes: &[u8]) -> Result<bool, String> {
    let downloads = std::env::var_os("USERPROFILE").map(|home| std::path::PathBuf::from(home).join("Downloads"));
    let mut dialog = rfd::FileDialog::new().set_file_name(suggested);
    if let Some(folder) = downloads.filter(|folder| folder.is_dir()) {
        dialog = dialog.set_directory(folder);
    }
    let Some(path) = dialog.save_file() else {
        return Ok(false);
    };
    std::fs::write(path, bytes).map(|()| true).map_err(|error| error.to_string())
}

pub fn pick_file() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new().pick_file()
}

/// Whether Ctrl+V is down right now. The window system swallows that key when the clipboard has
/// no text, which is exactly when a picture is being pasted, so the keyboard is asked directly.
#[cfg(windows)]
pub fn paste_keys_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL};
    // SAFETY: reads the state of two keys, with no pointers involved.
    let down = |key: i32| unsafe { GetAsyncKeyState(key) } < 0;
    down(i32::from(VK_CONTROL.0)) && down(i32::from(b'V'))
}

#[cfg(not(windows))]
pub fn paste_keys_down() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(width: u32, height: u32) -> DynamicImage {
        let mut seed = 12345u32;
        DynamicImage::ImageRgb8(image::RgbImage::from_fn(width, height, |_, _| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            image::Rgb([(seed >> 24) as u8, (seed >> 16) as u8, (seed >> 8) as u8])
        }))
    }

    #[test]
    fn a_picture_is_shrunk_until_it_fits_in_a_message() {
        let small = encode_image(&noise(200, 100)).unwrap();
        assert_eq!((small.width, small.height), (200, 100));
        assert!(is_webp(&small.data));

        // Pure noise does not compress, so only a smaller picture gets under the limit.
        let large = encode_image(&noise(2400, 1800)).unwrap();
        assert!(large.data.len() <= MAX_IMAGE_BYTES);
        assert!(large.width < 2400 && large.width * 3 == large.height * 4);
    }

    #[test]
    fn a_small_gif_stays_a_gif_and_a_large_one_goes_as_a_file() {
        let mut bytes = Vec::new();
        noise(40, 30).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Gif).unwrap();
        let Attachment::Image { image, .. } = from_bytes("a.gif", bytes.clone()).unwrap() else { panic!("a picture") };
        assert_eq!((image.mime.as_str(), image.width, image.height), ("image/gif", 40, 30));

        bytes.resize(MAX_IMAGE_BYTES + 1, 0);
        let Attachment::File(file) = from_bytes("grande.gif", bytes).unwrap() else { panic!("a file") };
        assert_eq!(file.mime, "image/gif");
    }

    #[test]
    fn anything_else_goes_as_a_file_with_its_kind() {
        let Attachment::File(file) = from_bytes("notas.txt", b"oi".to_vec()).unwrap() else { panic!("a file") };
        assert_eq!(file.mime, "text/plain");
        assert!(from_bytes("vazio.txt", Vec::new()).is_err());
    }

    #[test]
    fn a_large_file_is_sent_from_where_it_is_on_the_disk() {
        let folder = tempfile::tempdir().unwrap();
        let small = folder.path().join("pequeno.bin");
        std::fs::write(&small, vec![1u8; 1000]).unwrap();
        let Attachment::File(file) = from_path(&small).unwrap() else { panic!("a file") };
        assert_eq!((file.data.len(), file.path.is_none(), file.size()), (1000, true, 1000));

        // Above what the message servers carry, the bytes stay on the disk until they go.
        let large = folder.path().join("grande.zip");
        std::fs::write(&large, vec![1u8; MAX_RELAY_BYTES + 1]).unwrap();
        let attachment = from_path(&large).unwrap();
        assert_eq!(attachment.size(), MAX_RELAY_BYTES + 1);
        let Attachment::File(file) = attachment else { panic!("a file") };
        assert_eq!((file.data.len(), file.path.as_deref(), file.mime.as_str()), (0, Some(large.as_path()), "application/zip"));

        std::fs::write(folder.path().join("vazio.bin"), b"").unwrap();
        assert!(from_path(&folder.path().join("vazio.bin")).is_err());
        assert!(from_path(&folder.path().join("nao existe")).is_err());
    }
}
