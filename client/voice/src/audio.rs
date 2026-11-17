//! The microphone and the speakers. Everything in between runs at 48 kHz mono in pieces of 10 ms,
//! which is what the voice processing and the connections work with.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use libwebrtc::native::apm::AudioProcessingModule;
use tokio::sync::mpsc;

pub const RATE: u32 = 48_000;
/// Samples in 10 ms.
pub const FRAME: usize = 480;
/// How much voice of one person may wait to be played before the oldest is dropped.
const MAX_QUEUED: usize = FRAME * 30;
/// What a queue is brought back to when it grew past the limit, so the delay does not stay.
const TRIMMED: usize = FRAME * 6;

/// Which microphone and which speakers the person picked, by the name Windows gives them. With
/// none picked, or when the one picked is gone, the default of Windows is used.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceChoice {
    pub input: Option<String>,
    pub output: Option<String>,
}

static CHOICE: Mutex<DeviceChoice> = Mutex::new(DeviceChoice { input: None, output: None });

/// Sets the devices for the calls and tunes that start from now on.
pub fn choose_devices(choice: DeviceChoice) {
    *CHOICE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = choice;
}

pub fn chosen_devices() -> DeviceChoice {
    CHOICE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
}

/// The names of the microphones and of the speakers plugged in now.
pub fn device_names() -> (Vec<String>, Vec<String>) {
    let host = cpal::default_host();
    let names = |devices: Option<Vec<cpal::Device>>| -> Vec<String> {
        let mut names: Vec<String> = devices.unwrap_or_default().iter().filter_map(name_of).collect();
        names.sort();
        names.dedup();
        names
    };
    (names(host.input_devices().ok().map(Iterator::collect)), names(host.output_devices().ok().map(Iterator::collect)))
}

fn name_of(device: &cpal::Device) -> Option<String> {
    device.description().ok().map(|description| description.name().to_owned())
}

pub(crate) fn output_device(host: &cpal::Host) -> Option<cpal::Device> {
    let wanted = chosen_devices().output;
    let picked = wanted.and_then(|wanted| host.output_devices().ok()?.find(|device| name_of(device).as_deref() == Some(wanted.as_str())));
    picked.or_else(|| host.default_output_device())
}

fn input_device(host: &cpal::Host) -> Option<cpal::Device> {
    let wanted = chosen_devices().input;
    let picked = wanted.and_then(|wanted| host.input_devices().ok()?.find(|device| name_of(device).as_deref() == Some(wanted.as_str())));
    picked.or_else(|| host.default_input_device())
}

/// The voice processing of WebRTC, shared by the capture and the playback threads.
pub struct Processing(Mutex<AudioProcessingModule>);

// SAFETY: the module is only touched under the mutex, and WebRTC documents it as usable from the
// capture and render threads.
unsafe impl Send for Processing {}
unsafe impl Sync for Processing {}

impl Processing {
    pub fn new() -> Self {
        let mut module = AudioProcessingModule::new(true, true, true, true);
        let _ = module.set_stream_delay_ms(60);
        Self(Mutex::new(module))
    }

    /// Cleans one piece of what the microphone heard: echo, noise and level.
    pub fn capture(&self, frame: &mut [i16]) {
        let mut module = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = module.process_stream(frame, RATE as i32, 1);
    }

    /// Shows the module what is about to be played, which is what it removes from the microphone.
    pub fn playback(&self, frame: &mut [i16]) {
        let mut module = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = module.process_reverse_stream(frame, RATE as i32, 1);
    }
}

impl Default for Processing {
    fn default() -> Self {
        Self::new()
    }
}

struct Voice {
    queue: VecDeque<i16>,
    volume: f32,
}

/// A sound of the soundboard on its way out. The speakers and the microphone take it at their
/// own pace, so each has its own place in it.
struct Playing {
    samples: Arc<[i16]>,
    volume: f32,
    heard: usize,
    sent: usize,
    started: std::time::Instant,
}

/// Adds up the voices of everyone in the call for the speakers.
#[derive(Default)]
pub struct Mixer {
    voices: Mutex<HashMap<String, Voice>>,
    deaf: AtomicBool,
    sounds: Mutex<Vec<Playing>>,
}

impl Mixer {
    /// Starts a sound of the soundboard: the others hear it mixed with the microphone, and it
    /// plays here unless the person is deaf in the call.
    pub fn play_sound(&self, samples: Arc<[i16]>, volume: f32) {
        let mut sounds = self.sounds.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        sounds.push(Playing { samples, volume: volume.clamp(0.0, 1.0), heard: 0, sent: 0, started: std::time::Instant::now() });
    }

    /// Adds the next piece of every sound to `frame`, for the microphone when `to_mic` is true
    /// and for the speakers otherwise. A side that is not `audible` still moves along the sound.
    pub fn add_sounds(&self, frame: &mut [i16], to_mic: bool, audible: bool) {
        let mut sounds = self.sounds.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for sound in sounds.iter_mut() {
            let place = if to_mic { &mut sound.sent } else { &mut sound.heard };
            let piece = &sound.samples[(*place).min(sound.samples.len())..];
            if audible {
                for (slot, sample) in frame.iter_mut().zip(piece) {
                    let sum = f32::from(*slot) + f32::from(*sample) * sound.volume;
                    *slot = sum.clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16;
                }
            }
            *place += frame.len();
        }
        // A sound is over when both sides took all of it, or when one side never came for it,
        // as with no microphone plugged in.
        sounds.retain(|sound| {
            let length = std::time::Duration::from_secs_f64(sound.samples.len() as f64 / f64::from(RATE));
            let taken = sound.heard >= sound.samples.len() && sound.sent >= sound.samples.len();
            !taken && sound.started.elapsed() < length + std::time::Duration::from_secs(2)
        });
    }

    pub fn push(&self, id: &str, samples: &[i16]) {
        let mut voices = self.voices.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let voice = voices.entry(id.to_owned()).or_insert_with(|| Voice { queue: VecDeque::new(), volume: 1.0 });
        voice.queue.extend(samples);
        if voice.queue.len() > MAX_QUEUED {
            let excess = voice.queue.len() - TRIMMED;
            voice.queue.drain(..excess);
        }
    }

    pub fn remove(&self, id: &str) {
        self.voices.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(id);
    }

    pub fn clear(&self) {
        self.voices.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
    }

    /// The volume of one person, where 1 is as it arrives and 2 is the most.
    pub fn set_volume(&self, id: &str, volume: f32) {
        let mut voices = self.voices.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let voice = voices.entry(id.to_owned()).or_insert_with(|| Voice { queue: VecDeque::new(), volume: 1.0 });
        voice.volume = volume.clamp(0.0, 2.0);
    }

    pub fn set_deaf(&self, deaf: bool) {
        self.deaf.store(deaf, Ordering::Relaxed);
    }

    /// Fills `out` with the sum of what is waiting. Silence where nobody is speaking.
    fn mix(&self, out: &mut [i16]) {
        out.fill(0);
        let deaf = self.deaf.load(Ordering::Relaxed);
        let mut voices = self.voices.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for voice in voices.values_mut() {
            for slot in out.iter_mut() {
                let Some(sample) = voice.queue.pop_front() else { break };
                if !deaf {
                    let sum = f32::from(*slot) + f32::from(sample) * voice.volume;
                    *slot = sum.clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16;
                }
            }
        }
        drop(voices);
        self.add_sounds(out, false, !deaf);
    }
}

/// How loud a piece of sound is, from 0 to 1.
pub fn level_of(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|sample| f64::from(*sample) * f64::from(*sample)).sum();
    ((sum / samples.len() as f64).sqrt() / f64::from(i16::MAX)) as f32
}

/// Changes the rate of a mono signal by drawing a line between neighbors. Good enough for the
/// few devices that do not run at 48 kHz.
pub(crate) struct Resampler {
    step: f64,
    position: f64,
    last: f32,
}

impl Resampler {
    pub(crate) fn new(from: u32, to: u32) -> Self {
        Self { step: f64::from(from) / f64::from(to), position: 0.0, last: 0.0 }
    }

    pub(crate) fn push(&mut self, input: &[f32], output: &mut Vec<f32>) {
        for sample in input {
            while self.position < 1.0 {
                output.push(self.last + (*sample - self.last) * self.position as f32);
                self.position += self.step;
            }
            self.position -= 1.0;
            self.last = *sample;
        }
    }
}

fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
}

/// The microphone level, kept as the bits of an `f32` so any thread reads it without a lock.
#[derive(Default)]
pub struct Level(AtomicU32);

impl Level {
    pub fn set(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }

    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

/// The microphone alone, to show how loud it arrives, as the test of the settings. The level is
/// the loudness of the last pieces times six, at most 1, as `startMicTest` showed it.
pub struct MicTest {
    level: Arc<Level>,
    problem: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MicTest {
    /// Opens the microphone picked in the settings, or the default of Windows.
    pub fn start() -> Self {
        let level = Arc::new(Level::default());
        let problem = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let (heard, failed, stopped) = (Arc::clone(&level), Arc::clone(&problem), Arc::clone(&stop));
        let thread = std::thread::Builder::new()
            .name("telinha-mic-test".into())
            .spawn(move || {
                let host = cpal::default_host();
                let stream = open_meter(&host, heard);
                if let Err(error) = &stream {
                    *failed.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error.clone());
                }
                while !stopped.load(Ordering::Relaxed) {
                    std::thread::park_timeout(std::time::Duration::from_millis(200));
                }
                drop(stream);
            })
            .ok();
        Self { level, problem, stop, thread }
    }

    /// From 0 to 1.
    pub fn level(&self) -> f32 {
        self.level.get()
    }

    /// Why the microphone did not open, when it did not.
    pub fn problem(&self) -> Option<String> {
        self.problem.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }
}

impl Drop for MicTest {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

/// A stream of the microphone that only measures it.
fn open_meter(host: &cpal::Host, level: Arc<Level>) -> Result<cpal::Stream, String> {
    let device = input_device(host).ok_or("Nenhum microfone encontrado.")?;
    let supported = device.default_input_config().map_err(|_| "Não consegui abrir o microfone.".to_owned())?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let measure = move |sum: f64, count: usize| {
        if count > 0 {
            level.set(((sum / count as f64).sqrt() * 6.0).min(1.0) as f32);
        }
    };
    let failed = |error: cpal::Error| eprintln!("telinha: teste do microfone: {error}");
    let stream = match format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| measure(data.iter().map(|sample| f64::from(*sample).powi(2)).sum(), data.len()),
            failed,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                measure(data.iter().map(|sample| (f64::from(*sample) / f64::from(i16::MAX)).powi(2)).sum(), data.len());
            },
            failed,
            None,
        ),
        _ => return Err("Não consegui abrir o microfone.".to_owned()),
    }
    .map_err(|error| {
        let text = error.to_string();
        if text.contains("denied") || text.contains("negado") {
            "O Windows bloqueou o acesso ao microfone. Libere em Configurações > Privacidade e segurança.".to_owned()
        } else {
            "Não consegui abrir o microfone.".to_owned()
        }
    })?;
    stream.play().map_err(|_| "O microfone está em uso por outro programa.".to_owned())?;
    Ok(stream)
}

pub struct AudioIo {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AudioIo {
    /// Opens the default microphone and speakers. Each cleaned piece of 10 ms of the microphone
    /// goes to `captured`. A sentence about what could not be opened goes to `problems`.
    pub fn start(
        processing: Arc<Processing>,
        mixer: Arc<Mixer>,
        mic_on: Arc<AtomicBool>,
        mic_level: Arc<Level>,
        captured: mpsc::UnboundedSender<Vec<i16>>,
        problems: mpsc::UnboundedSender<String>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        // The streams of the sound library must stay on the thread that opened them.
        let thread = std::thread::Builder::new()
            .name("telinha-audio".into())
            .spawn(move || {
                let host = cpal::default_host();
                let output = open_output(&host, Arc::clone(&processing), Arc::clone(&mixer));
                if let Err(error) = &output {
                    let _ = problems.send(format!("não consegui abrir a saída de som: {error}"));
                }
                let input = open_input(&host, processing, mixer, mic_on, mic_level, captured);
                if let Err(error) = &input {
                    let _ = problems.send(format!("não consegui abrir o microfone: {error}"));
                }
                while !stopped.load(Ordering::Relaxed) {
                    std::thread::park_timeout(std::time::Duration::from_millis(200));
                }
                drop(input);
                drop(output);
            })
            .ok();
        Self { stop, thread }
    }
}

impl Drop for AudioIo {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

fn open_output(host: &cpal::Host, processing: Arc<Processing>, mixer: Arc<Mixer>) -> Result<cpal::Stream, String> {
    let device = output_device(host).ok_or("nenhuma saída de som")?;
    let supported = device.default_output_config().map_err(|error| error.to_string())?;
    let channels = usize::from(supported.channels());
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate;

    let mut resampler = Resampler::new(RATE, rate);
    let mut ready: VecDeque<f32> = VecDeque::new();
    let mut frame = [0i16; FRAME];
    let mut resampled = Vec::new();
    let mut fill = move |wanted: usize, mut write: Box<dyn FnMut(usize, f32) + '_>| {
        while ready.len() < wanted {
            mixer.mix(&mut frame);
            processing.playback(&mut frame);
            let floats: Vec<f32> = frame.iter().map(|sample| f32::from(*sample) / f32::from(i16::MAX)).collect();
            if rate == RATE {
                ready.extend(floats);
            } else {
                resampled.clear();
                resampler.push(&floats, &mut resampled);
                ready.extend(resampled.iter().copied());
            }
        }
        for index in 0..wanted {
            write(index, ready.pop_front().unwrap_or(0.0));
        }
    };
    let failed = |error: cpal::Error| eprintln!("telinha: saida de som: {error}");
    let stream = match format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            config.clone(),
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let frames = data.len() / channels;
                fill(frames, Box::new(|index, sample| data[index * channels..(index + 1) * channels].fill(sample)));
            },
            failed,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream(
            config.clone(),
            move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                let frames = data.len() / channels;
                fill(frames, Box::new(|index, sample| data[index * channels..(index + 1) * channels].fill(to_i16(sample))));
            },
            failed,
            None,
        ),
        other => return Err(format!("formato de som sem suporte: {other:?}")),
    }
    .map_err(|error| error.to_string())?;
    stream.play().map_err(|error| error.to_string())?;
    Ok(stream)
}

fn open_input(
    host: &cpal::Host,
    processing: Arc<Processing>,
    mixer: Arc<Mixer>,
    mic_on: Arc<AtomicBool>,
    mic_level: Arc<Level>,
    captured: mpsc::UnboundedSender<Vec<i16>>,
) -> Result<cpal::Stream, String> {
    let device = input_device(host).ok_or("nenhum microfone")?;
    let supported = device.default_input_config().map_err(|error| error.to_string())?;
    let channels = usize::from(supported.channels());
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate;

    let mut resampler = Resampler::new(rate, RATE);
    let mut mono = Vec::new();
    let mut resampled = Vec::new();
    let mut pending: Vec<i16> = Vec::with_capacity(FRAME * 4);
    let mut take = move |frames: usize, read: &dyn Fn(usize) -> f32| {
        mono.clear();
        for frame in 0..frames {
            let sum: f32 = (0..channels).map(|channel| read(frame * channels + channel)).sum();
            mono.push(sum / channels as f32);
        }
        let samples: &[f32] = if rate == RATE {
            &mono
        } else {
            resampled.clear();
            resampler.push(&mono, &mut resampled);
            &resampled
        };
        pending.extend(samples.iter().map(|sample| to_i16(*sample)));
        while pending.len() >= FRAME {
            let mut frame: Vec<i16> = pending.drain(..FRAME).collect();
            processing.capture(&mut frame);
            if mic_on.load(Ordering::Relaxed) {
                mic_level.set(level_of(&frame));
            } else {
                // A muted microphone still feeds the connection, with silence, so it stays alive.
                frame.fill(0);
                mic_level.set(0.0);
            }
            // The soundboard goes out after the cleaning, which would take it for noise, and
            // also with the microphone muted.
            mixer.add_sounds(&mut frame, true, true);
            let _ = captured.send(frame);
        }
    };
    let failed = |error: cpal::Error| eprintln!("telinha: microfone: {error}");
    let stream = match format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            config.clone(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| take(data.len() / channels, &|index| data[index]),
            failed,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            config.clone(),
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                take(data.len() / channels, &|index| f32::from(data[index]) / f32::from(i16::MAX));
            },
            failed,
            None,
        ),
        other => return Err(format!("formato de som sem suporte: {other:?}")),
    }
    .map_err(|error| error.to_string())?;
    stream.play().map_err(|error| error.to_string())?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn showing_the_echo_canceller_what_plays_does_not_change_it() {
        let processing = Processing::new();
        // A noisy, loud sound of half a second, as a soundboard sound often is.
        let mut seed = 12345u32;
        let mut changed = 0usize;
        let mut total = 0usize;
        for _ in 0..50 {
            let original: Vec<i16> = (0..FRAME)
                .map(|_| {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    ((seed >> 16) as i16 / 3).saturating_add(0)
                })
                .collect();
            let mut frame = original.clone();
            processing.playback(&mut frame);
            changed += frame.iter().zip(&original).filter(|(a, b)| a != b).count();
            total += frame.len();
        }
        println!("changed {changed} of {total} samples");
        assert_eq!(changed, 0, "the echo canceller changed what goes to the speakers");
    }

    #[test]
    fn voices_are_added_up_with_each_ones_volume_and_never_overflow() {
        let mixer = Mixer::default();
        mixer.push("a", &[1000, 1000, 30000]);
        mixer.push("b", &[500, -2000, 30000]);
        mixer.set_volume("b", 2.0);
        let mut out = [0i16; 4];
        mixer.mix(&mut out);
        assert_eq!(out, [2000, -3000, i16::MAX, 0]);

        mixer.push("a", &[100; 4]);
        mixer.set_deaf(true);
        mixer.mix(&mut out);
        assert_eq!(out, [0; 4]);
        mixer.set_deaf(false);
        mixer.mix(&mut out);
        assert_eq!(out, [0; 4], "what arrived while deaf is not played later");
    }

    #[test]
    fn a_sound_reaches_the_speakers_and_the_microphone_each_at_its_own_pace() {
        let mixer = Mixer::default();
        let samples: Arc<[i16]> = (1..=1000).map(|value| value as i16).collect::<Vec<_>>().into();
        mixer.play_sound(Arc::clone(&samples), 0.5);
        let mut heard = [0i16; 480];
        mixer.mix(&mut heard);
        assert_eq!((heard[0], heard[1], heard[479]), (0, 1, 240), "half the volume");
        // The microphone has not taken anything yet, so it starts from the beginning, on top of
        // what it already carries.
        let mut sent = [100i16; 480];
        mixer.add_sounds(&mut sent, true, true);
        assert_eq!((sent[1], sent[479]), (101, 340));
        // A deaf person hears nothing but the sound still goes out, and both move along.
        mixer.set_deaf(true);
        mixer.mix(&mut heard);
        assert!(heard.iter().all(|sample| *sample == 0));
        mixer.set_deaf(false);
        mixer.add_sounds(&mut sent, true, true);
        mixer.mix(&mut heard);
        assert_eq!(heard[0], 480, "the third piece for the speakers, sample 961");
        assert_eq!(heard[40], 0, "past the end there is nothing");
        mixer.add_sounds(&mut sent, true, true);
        assert!(mixer.sounds.lock().unwrap().is_empty(), "both sides took it all");
    }

    #[test]
    fn a_voice_that_piles_up_loses_the_oldest_part() {
        let mixer = Mixer::default();
        mixer.push("a", &vec![1; MAX_QUEUED + FRAME]);
        let voices = mixer.voices.lock().unwrap();
        assert_eq!(voices["a"].queue.len(), TRIMMED);
    }

    /// Run by hand: `cargo test -p telinha-voice devices -- --ignored --nocapture`. Only lists
    /// what is plugged in; nothing is opened.
    #[test]
    #[ignore]
    fn the_devices_of_this_computer_have_names() {
        let (inputs, outputs) = device_names();
        println!("microphones: {inputs:?}\nspeakers: {outputs:?}");
        assert!(!outputs.is_empty());
        let host = cpal::default_host();
        choose_devices(DeviceChoice { input: None, output: Some("a device that is not there".into()) });
        assert_eq!(output_device(&host).and_then(|device| name_of(&device)), host.default_output_device().and_then(|device| name_of(&device)));
        choose_devices(DeviceChoice { input: None, output: outputs.last().cloned() });
        assert_eq!(output_device(&host).and_then(|device| name_of(&device)), outputs.last().cloned());
        choose_devices(DeviceChoice::default());
    }

    #[test]
    fn the_rate_change_keeps_the_length_in_proportion() {
        let mut resampler = Resampler::new(44_100, 48_000);
        let mut out = Vec::new();
        resampler.push(&vec![0.5; 4410], &mut out);
        assert!((out.len() as i64 - 4800).abs() <= 2);
        assert!(out[100..].iter().all(|sample| (*sample - 0.5).abs() < 1e-3));
        assert!(level_of(&[i16::MAX; 10]) > 0.99 && level_of(&[0; 10]) == 0.0);
    }
}
