//! Pictures for the window: the last frame of a video, turned into RGBA once the window took the
//! one before, so a video nobody looks at costs no conversion.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use libwebrtc::video_frame::{VideoBuffer, VideoFormatType};
use libwebrtc::video_stream::native::NativeVideoStream;
use libwebrtc::video_track::RtcVideoTrack;

/// Asks the window to draw again.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// One frame, four bytes a pixel in the order red, green, blue, alpha.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The last frame of one video.
pub struct FrameStore {
    latest: Mutex<Option<Arc<Picture>>>,
    serial: AtomicU64,
    /// True once the window took the last frame, so the next one is worth converting.
    taken: AtomicBool,
    waker: Waker,
    /// The size the window draws the video at, in pixels, width above and height below. Zero is
    /// the size the video comes in. Frames are made smaller to cover it, never larger.
    wanted: AtomicU64,
    /// The largest size asked for in the pass of the window being drawn now, and that pass.
    /// The same video can be drawn twice in one pass, small in its tile and large over the
    /// window, and the size must not drop to the small one between the two.
    asked: Mutex<(u64, u64)>,
    /// The least time between two frames converted, for a video that does not need them all.
    gap: Duration,
    last: Mutex<Option<Instant>>,
    /// The colors come in BT.709, as the screens of the capture engine do; cameras use BT.601.
    bt709: bool,
}

impl FrameStore {
    pub fn new(waker: Waker) -> Arc<Self> {
        Self::with_gap(waker, Duration::ZERO)
    }

    /// A store that converts at most one frame in each `gap`.
    pub fn with_gap(waker: Waker, gap: Duration) -> Arc<Self> {
        Arc::new(Self {
            latest: Mutex::new(None),
            serial: AtomicU64::new(0),
            taken: AtomicBool::new(true),
            waker,
            wanted: AtomicU64::new(0),
            asked: Mutex::new((u64::MAX, 0)),
            gap,
            last: Mutex::new(None),
            bt709: false,
        })
    }

    /// A store for a shared screen: its colors are BT.709, not the BT.601 of a camera.
    pub fn screen(waker: Waker, gap: Duration) -> Arc<Self> {
        let mut store = Self::with_gap(waker, gap);
        Arc::get_mut(&mut store).expect("a new store").bt709 = true;
        store
    }

    /// True when a new frame would be looked at.
    pub fn wants(&self) -> bool {
        self.taken.load(Ordering::Relaxed) && self.gap_passed()
    }

    fn gap_passed(&self) -> bool {
        if self.gap.is_zero() {
            return true;
        }
        let mut last = self.last.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if last.is_some_and(|last| last.elapsed() < self.gap) {
            return false;
        }
        *last = Some(Instant::now());
        true
    }

    /// The size the window draws the video at now, in pixels, in its pass `pass`. With more than
    /// one place drawing it, the frames cover the largest. The size only gets smaller once a
    /// whole pass asked for less.
    pub fn want_size(&self, width: u32, height: u32, pass: u64) {
        let pack = |width: u64, height: u64| (width << 32) | height;
        let size = |packed: u64| (packed >> 32, packed & 0xffff_ffff);
        let (width, height) = (u64::from(width), u64::from(height));
        let mut asked = self.asked.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let wanted = if asked.0 == pass {
            let (before_width, before_height) = size(asked.1);
            pack(before_width.max(width), before_height.max(height))
        } else {
            // A new pass: what the last one asked for holds until this one asks for more.
            if asked.0 != u64::MAX {
                self.wanted.store(asked.1, Ordering::Relaxed);
            }
            pack(width, height)
        };
        *asked = (pass, wanted);
        let (now_width, now_height) = size(self.wanted.load(Ordering::Relaxed));
        let (wanted_width, wanted_height) = size(wanted);
        if asked.0 == pass && (wanted_width > now_width || wanted_height > now_height || self.wanted.load(Ordering::Relaxed) == 0) {
            self.wanted.store(pack(now_width.max(wanted_width), now_height.max(wanted_height)), Ordering::Relaxed);
        }
    }

    /// The size to convert a frame of this size to: the smallest that still covers what the
    /// window draws, in even numbers.
    fn fit(&self, width: u32, height: u32) -> (u32, u32) {
        let wanted = self.wanted.load(Ordering::Relaxed);
        let (box_width, box_height) = ((wanted >> 32) as u32, wanted as u32);
        if box_width == 0 || box_height == 0 {
            return (width, height);
        }
        let scale = (box_width as f32 / width as f32).max(box_height as f32 / height as f32);
        if scale >= 0.9 {
            return (width, height);
        }
        let even = |value: f32| ((value.round() as u32).max(2) + 1) & !1;
        (even(width as f32 * scale), even(height as f32 * scale))
    }

    pub fn put(&self, picture: Picture) {
        *self.latest.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::new(picture));
        self.taken.store(false, Ordering::Relaxed);
        self.serial.fetch_add(1, Ordering::Relaxed);
        (self.waker)();
    }

    /// The last frame and its number, which changes with every new frame.
    pub fn latest(&self) -> Option<(u64, Arc<Picture>)> {
        let picture = self.latest.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()?;
        self.taken.store(true, Ordering::Relaxed);
        Some((self.serial.load(Ordering::Relaxed), picture))
    }

    /// Forgets the frame, for a video that stopped.
    pub fn clear(&self) {
        *self.latest.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        self.taken.store(true, Ordering::Relaxed);
        self.serial.fetch_add(1, Ordering::Relaxed);
        (self.waker)();
    }

    /// Turns a frame of libwebrtc into a picture, when the window is ready for one.
    pub fn put_buffer(&self, buffer: &dyn VideoBuffer) {
        if !self.wants() {
            return;
        }
        let (full_width, full_height) = (buffer.width(), buffer.height());
        if full_width == 0 || full_height == 0 {
            return;
        }
        let (width, height) = self.fit(full_width, full_height);
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        if self.bt709 {
            let i420 = if (width, height) == (full_width, full_height) { buffer.to_i420() } else { buffer.to_i420().scale(width as i32, height as i32) };
            let (y, u, v) = i420.data();
            let (stride_y, stride_u, stride_v) = i420.strides();
            // SAFETY: the planes and strides come from the same buffer, which lives until the
            // call returns, and `rgba` holds exactly `width * height` pixels of four bytes.
            let turned = unsafe {
                webrtc_sys::yuv_helper::ffi::h420_to_abgr(
                    y.as_ptr(),
                    stride_y as i32,
                    u.as_ptr(),
                    stride_u as i32,
                    v.as_ptr(),
                    stride_v as i32,
                    rgba.as_mut_ptr(),
                    (width * 4) as i32,
                    width as i32,
                    height as i32,
                )
            };
            if turned.is_ok() {
                self.put(Picture { width, height, rgba });
            }
            return;
        }
        // In the names of libyuv, ABGR is red first in memory.
        if (width, height) == (full_width, full_height) {
            buffer.to_argb(VideoFormatType::ABGR, &mut rgba, width * 4, width as i32, height as i32);
        } else {
            // A tile is much smaller than a screen: the frame is made small before the colors are
            // turned, which is most of the work.
            let small = buffer.to_i420().scale(width as i32, height as i32);
            (&small as &dyn VideoBuffer).to_argb(VideoFormatType::ABGR, &mut rgba, width * 4, width as i32, height as i32);
        }
        self.put(Picture { width, height, rgba });
    }

    /// Turns a frame of the camera, in NV12, into a picture, when the window is ready for one.
    pub fn put_nv12(&self, frame: &Nv12<'_>) {
        if !self.wants() || frame.width == 0 || frame.height == 0 {
            return;
        }
        let mut rgba = vec![0u8; (frame.width * frame.height * 4) as usize];
        libwebrtc::native::yuv_helper::nv12_to_abgr(
            frame.y,
            frame.stride_y,
            frame.uv,
            frame.stride_uv,
            &mut rgba,
            frame.width * 4,
            frame.width as i32,
            frame.height as i32,
        );
        self.put(Picture { width: frame.width, height: frame.height, rgba });
    }
}

/// A frame in NV12 as a camera hands it: a plane of brightness and one of color, half size.
pub struct Nv12<'a> {
    pub width: u32,
    pub height: u32,
    pub y: &'a [u8],
    pub stride_y: u32,
    pub uv: &'a [u8],
    pub stride_uv: u32,
}

/// Sends a frame of the camera into a call.
pub fn send_nv12(source: &libwebrtc::video_source::native::NativeVideoSource, frame: &Nv12<'_>, timestamp_us: i64) {
    use libwebrtc::video_frame::{NV12Buffer, VideoFrame, VideoRotation};
    let (width, height) = (frame.width & !1, frame.height & !1);
    if width == 0 || height == 0 {
        return;
    }
    let mut buffer = NV12Buffer::new(width, height);
    let (stride_y, stride_uv) = buffer.strides();
    let (y, uv) = buffer.data_mut();
    for row in 0..height as usize {
        let from = &frame.y[row * frame.stride_y as usize..][..width as usize];
        y[row * stride_y as usize..][..width as usize].copy_from_slice(from);
    }
    for row in 0..(height / 2) as usize {
        let from = &frame.uv[row * frame.stride_uv as usize..][..width as usize];
        uv[row * stride_uv as usize..][..width as usize].copy_from_slice(from);
    }
    source.capture_frame(&VideoFrame { rotation: VideoRotation::VideoRotation0, timestamp_us, frame_metadata: None, buffer });
}

/// Reads a track into a store until the track ends.
pub fn watch_track(track: RtcVideoTrack, store: Arc<FrameStore>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut stream = NativeVideoStream::new(track);
        while let Some(frame) = stream.next().await {
            store.put_buffer(frame.buffer.as_ref());
        }
        store.clear();
    })
}

/// Where the frames of the camera of this app go: the preview in the window and, during a call,
/// the call. Cloned into the thread of the camera.
#[derive(Clone)]
pub struct CameraFeed {
    preview: Arc<FrameStore>,
    call: Arc<Mutex<Option<libwebrtc::video_source::native::NativeVideoSource>>>,
    started: std::time::Instant,
}

impl CameraFeed {
    pub fn new(waker: Waker) -> Self {
        Self { preview: FrameStore::new(waker), call: Arc::default(), started: std::time::Instant::now() }
    }

    /// The last frame of the camera, for the window.
    pub fn preview(&self) -> &Arc<FrameStore> {
        &self.preview
    }

    /// Sends the frames into this call too, or into none.
    pub fn set_call(&self, call: Option<&crate::Call>) {
        *self.call.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = call.map(crate::Call::camera);
    }

    /// What the camera hands each frame to.
    pub fn sink(&self) -> crate::camera::Sink {
        let feed = self.clone();
        Box::new(move |frame| {
            feed.preview.put_nv12(frame);
            if let Some(source) = feed.call.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref() {
                send_nv12(source, frame, feed.started.elapsed().as_micros() as i64);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_video_drawn_small_and_large_in_one_pass_keeps_the_large_size() {
        let store = FrameStore::new(Arc::new(|| {}));
        // Pass 1: the tile asks for a small picture, then the full window for a large one.
        store.want_size(320, 180, 1);
        assert_eq!(store.fit(1920, 1080), (320, 180));
        store.want_size(1920, 1080, 1);
        assert_eq!(store.fit(1920, 1080), (1920, 1080));
        // Pass 2: the tile comes first again, and a frame converted between the two draws
        // still covers the full window.
        store.want_size(320, 180, 2);
        assert_eq!(store.fit(1920, 1080), (1920, 1080));
        store.want_size(1920, 1080, 2);
        // The full window closes: pass 3 asks only for the tile, and the size drops once that
        // whole pass is over.
        store.want_size(320, 180, 3);
        assert_eq!(store.fit(1920, 1080), (1920, 1080));
        store.want_size(320, 180, 4);
        assert_eq!(store.fit(1920, 1080), (320, 180));
    }
}
