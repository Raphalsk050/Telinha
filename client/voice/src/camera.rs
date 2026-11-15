//! The camera, through Media Foundation: the list of cameras, and a camera running on its own
//! thread that hands each frame over in NV12, at 1280×720 and 30 frames a second when it can,
//! as `captureTrack` in call.js asked the browser for.
//!
//! Blurring the background is asked of the camera itself. Only cameras with the effects of
//! Windows (Windows Studio Effects) can do it.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use crate::video::Nv12;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const FPS: u32 = 30;

/// What a camera answered about blurring: not known yet, it cannot, it can.
const BLUR_UNKNOWN: u8 = 0;
const BLUR_NO: u8 = 1;
const BLUR_YES: u8 = 2;

/// Where each frame goes.
pub type Sink = Box<dyn FnMut(&Nv12<'_>) + Send>;

/// A camera that is running. Dropping it closes the camera.
pub struct Camera {
    stop: Arc<AtomicBool>,
    blur: Arc<AtomicBool>,
    blur_support: Arc<AtomicU8>,
    problem: Arc<Mutex<Option<String>>>,
}

impl Camera {
    /// Opens the camera with this name, or the first one when it is gone or none is given.
    pub fn start(name: Option<String>, blur: bool, sink: Sink) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let blur = Arc::new(AtomicBool::new(blur));
        let blur_support = Arc::new(AtomicU8::new(BLUR_UNKNOWN));
        let problem = Arc::new(Mutex::new(None));
        let shared = (Arc::clone(&stop), Arc::clone(&blur), Arc::clone(&blur_support), Arc::clone(&problem));
        let spawned = std::thread::Builder::new().name("telinha-camera".into()).spawn(move || {
            let (stop, blur, blur_support, problem) = shared;
            if let Err(error) = imp::run(name, &stop, &blur, &blur_support, sink) {
                *problem.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error);
            }
        });
        if spawned.is_err() {
            *problem.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some("Não consegui abrir a câmera.".to_owned());
        }
        Self { stop, blur, blur_support, problem }
    }

    /// Turns the blur of the background on or off while the camera runs.
    pub fn set_blur(&self, on: bool) {
        self.blur.store(on, Ordering::Relaxed);
    }

    /// Whether the camera can blur the background. None until it opened.
    pub fn blur_supported(&self) -> Option<bool> {
        match self.blur_support.load(Ordering::Relaxed) {
            BLUR_NO => Some(false),
            BLUR_YES => Some(true),
            _ => None,
        }
    }

    /// Why the camera did not open or stopped, in words for the person.
    pub fn problem(&self) -> Option<String> {
        self.problem.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        // The thread sees it after its next frame and closes the camera there.
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// The names of the cameras plugged in now, in the order Windows lists them.
pub fn camera_names() -> Vec<String> {
    imp::names().unwrap_or_default()
}

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

    use windows::Win32::Media::KernelStreaming::{KSCAMERA_EXTENDEDPROP_BACKGROUNDSEGMENTATION_BLUR, KSPROPERTY_CAMERACONTROL_EXTENDED_BACKGROUNDSEGMENTATION};
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
    use windows::core::{GUID, Interface, PWSTR};

    use super::{BLUR_NO, BLUR_YES, FPS, HEIGHT, Sink, WIDTH};
    use crate::video::Nv12;

    const E_ACCESSDENIED: i32 = 0x8007_0005_u32 as i32;
    const DEVICE_IN_USE: [i32; 3] = [0xC00D_3704_u32 as i32, 0xC00D_3EA3_u32 as i32, 0x8007_00AA_u32 as i32];

    fn start() -> windows::core::Result<()> {
        unsafe {
            // A thread that already has COM in another mode keeps it, which is fine here.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_LITE)
        }
    }

    /// The cameras, with their names.
    fn devices() -> windows::core::Result<Vec<(String, IMFActivate)>> {
        start()?;
        unsafe {
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 1)?;
            let attributes = attributes.ok_or_else(windows::core::Error::empty)?;
            attributes.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)?;
            let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
            let mut count = 0u32;
            MFEnumDeviceSources(&attributes, &mut list, &mut count)?;
            let mut found = Vec::new();
            if !list.is_null() {
                for slot in std::slice::from_raw_parts_mut(list, count as usize) {
                    let Some(device) = slot.take() else { continue };
                    let mut text = PWSTR::null();
                    let mut length = 0u32;
                    let name = if device.GetAllocatedString(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &mut text, &mut length).is_ok() {
                        let name = text.to_string().unwrap_or_default();
                        CoTaskMemFree(Some(text.0 as _));
                        name
                    } else {
                        String::new()
                    };
                    found.push((name, device));
                }
                CoTaskMemFree(Some(list as _));
            }
            Ok(found)
        }
    }

    pub fn names() -> Option<Vec<String>> {
        Some(devices().ok()?.into_iter().map(|(name, _)| name).collect())
    }

    fn words(error: &windows::core::Error) -> String {
        let code = error.code().0;
        if code == E_ACCESSDENIED {
            "O Windows bloqueou o acesso à câmera. Libere em Configurações > Privacidade e segurança.".to_owned()
        } else if DEVICE_IN_USE.contains(&code) {
            "A câmera está em uso por outro programa.".to_owned()
        } else {
            "Não consegui abrir a câmera.".to_owned()
        }
    }

    /// The size and the rate of a type, and how close it is to what is wanted: the closest
    /// size, then at least 30 frames a second.
    fn score(media: &IMFMediaType) -> Option<(u32, u32, u64)> {
        unsafe {
            let size = media.GetUINT64(&MF_MT_FRAME_SIZE).ok()?;
            let (width, height) = ((size >> 32) as u32, size as u32);
            let rate = media.GetUINT64(&MF_MT_FRAME_RATE).ok().map_or(0, |rate| {
                let (numerator, denominator) = ((rate >> 32) as u32, (rate as u32).max(1));
                numerator / denominator
            });
            let distance = u64::from(width.abs_diff(WIDTH)) + u64::from(height.abs_diff(HEIGHT));
            let slow = if rate >= FPS { 0 } else { u64::from(FPS - rate) * 100 };
            Some((width, height, distance * 10 + slow))
        }
    }

    pub fn run(name: Option<String>, stop: &AtomicBool, blur: &AtomicBool, blur_support: &AtomicU8, mut sink: Sink) -> Result<(), String> {
        let devices = devices().map_err(|error| words(&error))?;
        let chosen = name.as_ref().and_then(|name| devices.iter().position(|(found, _)| found == name)).unwrap_or(0);
        let (_, device) = devices.get(chosen).ok_or("Nenhuma câmera encontrada.")?;
        unsafe {
            let source: IMFMediaSource = device.ActivateObject().map_err(|error| words(&error))?;
            let control = blur_control(&source);
            blur_support.store(if control.is_some() { BLUR_YES } else { BLUR_NO }, Ordering::Relaxed);
            let mut blurred = None;

            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 2).map_err(|error| words(&error))?;
            let attributes = attributes.ok_or("Não consegui abrir a câmera.")?;
            let _ = attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1);
            let _ = attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);
            let reader = MFCreateSourceReaderFromMediaSource(&source, &attributes).map_err(|error| words(&error))?;
            let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

            // The way of the camera closest to 720p at 30, then NV12 out of it.
            let mut best: Option<(IMFMediaType, u32, u32, u64)> = None;
            let mut index = 0;
            while let Ok(native) = reader.GetNativeMediaType(stream, index) {
                index += 1;
                if let Some((width, height, score)) = score(&native) {
                    if best.as_ref().is_none_or(|(_, _, _, best)| score < *best) {
                        best = Some((native, width, height, score));
                    }
                }
            }
            let (width, height) = match &best {
                Some((native, width, height, _)) => {
                    let _ = reader.SetCurrentMediaType(stream, None, native);
                    (*width, *height)
                }
                None => (WIDTH, HEIGHT),
            };
            let output = MFCreateMediaType().map_err(|error| words(&error))?;
            let _ = output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video);
            let _ = output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12);
            let _ = output.SetUINT64(&MF_MT_FRAME_SIZE, (u64::from(width) << 32) | u64::from(height));
            reader.SetCurrentMediaType(stream, None, &output).map_err(|error| words(&error))?;
            let current = reader.GetCurrentMediaType(stream).map_err(|error| words(&error))?;
            let size = current.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or((u64::from(width) << 32) | u64::from(height));
            let (width, height) = ((size >> 32) as u32, size as u32);
            let default_stride = current.GetUINT32(&MF_MT_DEFAULT_STRIDE).ok().map(|stride| stride as i32).filter(|stride| *stride > 0);

            while !stop.load(Ordering::Relaxed) {
                let wanted = blur.load(Ordering::Relaxed);
                if blurred != Some(wanted) {
                    if let Some(control) = &control {
                        let flags = if wanted { KSCAMERA_EXTENDEDPROP_BACKGROUNDSEGMENTATION_BLUR } else { 0 };
                        let _ = control.SetFlags(flags);
                        let _ = control.CommitSettings();
                    }
                    blurred = Some(wanted);
                }
                let mut flags = 0u32;
                let mut time = 0i64;
                let mut sample: Option<IMFSample> = None;
                reader.ReadSample(stream, 0, None, Some(&mut flags), Some(&mut time), Some(&mut sample)).map_err(|error| words(&error))?;
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    return Err("A câmera parou.".to_owned());
                }
                let Some(sample) = sample else { continue };
                let Ok(buffer) = sample.ConvertToContiguousBuffer() else { continue };
                deliver(&buffer, width, height, default_stride, &mut sink);
            }
            let _ = source.Shutdown();
        }
        Ok(())
    }

    /// Hands one frame to the sink, reading its rows with the pitch the buffer has.
    unsafe fn deliver(buffer: &IMFMediaBuffer, width: u32, height: u32, default_stride: Option<i32>, sink: &mut Sink) {
        unsafe {
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut pitch = 0i32;
            let planar = buffer.cast::<IMF2DBuffer>().ok();
            let locked_2d = planar.as_ref().is_some_and(|planar| planar.Lock2D(&mut data, &mut pitch).is_ok() && pitch > 0);
            let mut length = 0u32;
            if !locked_2d {
                if buffer.Lock(&mut data, None, Some(&mut length)).is_err() {
                    return;
                }
                pitch = default_stride.unwrap_or(width as i32);
            }
            let stride = pitch as usize;
            let luma = stride * height as usize;
            let total = luma + stride * (height as usize).div_ceil(2);
            if !data.is_null() && (locked_2d || length as usize >= total) {
                let bytes = std::slice::from_raw_parts(data, total);
                let frame = Nv12 { width, height, y: &bytes[..luma], stride_y: stride as u32, uv: &bytes[luma..], stride_uv: stride as u32 };
                sink(&frame);
            }
            if locked_2d {
                if let Some(planar) = planar {
                    let _ = planar.Unlock2D();
                }
            } else {
                let _ = buffer.Unlock();
            }
        }
    }

    /// The control of the blur, when the camera has one that can blur.
    fn blur_control(source: &IMFMediaSource) -> Option<IMFExtendedCameraControl> {
        unsafe {
            let service = source.cast::<IMFGetService>().ok()?;
            let controller: IMFExtendedCameraController = service.GetService(&GUID::zeroed()).ok()?;
            let control = controller
                .GetExtendedCameraControl(MF_CAPTURE_ENGINE_MEDIASOURCE.0, KSPROPERTY_CAMERACONTROL_EXTENDED_BACKGROUNDSEGMENTATION.0 as u32)
                .ok()?;
            (control.GetCapabilities() & KSCAMERA_EXTENDEDPROP_BACKGROUNDSEGMENTATION_BLUR != 0).then_some(control)
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::sync::atomic::{AtomicBool, AtomicU8};

    use super::Sink;

    pub fn names() -> Option<Vec<String>> {
        None
    }

    pub fn run(_: Option<String>, _: &AtomicBool, _: &AtomicBool, _: &AtomicU8, _: Sink) -> Result<(), String> {
        Err("Nenhuma câmera encontrada.".to_owned())
    }
}

#[cfg(test)]
mod tests {
    /// Run by hand: `cargo test -p telinha-voice camera -- --ignored --nocapture`. Lists the
    /// cameras and, when there is one, reads a few frames from the first.
    #[test]
    #[ignore]
    fn the_first_camera_gives_frames() {
        let names = super::camera_names();
        println!("cameras: {names:?}");
        if names.is_empty() {
            return;
        }
        let (sender, frames) = std::sync::mpsc::channel();
        let camera = super::Camera::start(None, false, Box::new(move |frame| {
            let _ = sender.send((frame.width, frame.height, frame.y.len(), frame.uv.len()));
        }));
        for _ in 0..10 {
            match frames.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(frame) => println!("frame {frame:?}"),
                Err(_) => panic!("no frame: {:?}", camera.problem()),
            }
        }
        println!("blur: {:?}", camera.blur_supported());
    }
}
