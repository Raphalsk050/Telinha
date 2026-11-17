//! The short tunes of the app: someone came into the room, someone left, a share started, a
//! call is ringing. Each is a handful of notes with a quick rise and a slow fade, the same
//! notes the app has always played.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use cpal::traits::{DeviceTrait, StreamTrait};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Sine,
    Triangle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tune {
    /// Each note is its pitch in hertz and how many seconds after the start it sounds.
    pub notes: Vec<(f32, f32)>,
    pub shape: Shape,
    /// How loud a note gets, where 1 is the loudest the speakers go.
    pub peak: f32,
    /// How long a note takes to fade, in seconds.
    pub length: f32,
}

impl Tune {
    fn steps(pitches: &[f32]) -> Self {
        Self {
            notes: pitches.iter().enumerate().map(|(index, pitch)| (*pitch, index as f32 * 0.12)).collect(),
            shape: Shape::Sine,
            // Loud enough to hear over a game or a call.
            peak: 0.45,
            length: 0.28,
        }
    }

    /// Two quick short notes, for the switches of the person's own sound.
    fn blip(pitches: &[f32], shape: Shape) -> Self {
        Self {
            notes: pitches.iter().enumerate().map(|(index, pitch)| (*pitch, index as f32 * 0.08)).collect(),
            shape,
            peak: 0.4,
            length: 0.16,
        }
    }

    /// The microphone went silent: a step down.
    pub fn mic_off() -> Self {
        Self::blip(&[783.99, 523.25], Shape::Sine)
    }

    /// The microphone came back: a step up.
    pub fn mic_on() -> Self {
        Self::blip(&[523.25, 783.99], Shape::Sine)
    }

    /// The person stopped hearing the call: lower, rounder and three steps down.
    pub fn deaf_on() -> Self {
        Self::blip(&[440.0, 349.23, 261.63], Shape::Triangle)
    }

    /// The person hears the call again: the same three steps up.
    pub fn deaf_off() -> Self {
        Self::blip(&[261.63, 349.23, 440.0], Shape::Triangle)
    }

    /// A message arrived from someone.
    pub fn message() -> Self {
        Self { notes: vec![(1046.5, 0.0), (1567.98, 0.07)], shape: Shape::Sine, peak: 0.3, length: 0.22 }
    }

    pub fn join() -> Self {
        Self::steps(&[587.33, 880.0])
    }

    pub fn leave() -> Self {
        Self::steps(&[659.25, 440.0])
    }

    /// Someone in the room started sharing their screen.
    pub fn live() -> Self {
        Self::steps(&[523.25, 783.99, 1046.5])
    }

    /// Someone started watching the share of this app.
    pub fn viewer() -> Self {
        Self::steps(&[880.0, 1318.51])
    }

    /// One "ding-dong, ding-dong" of a call coming in.
    pub fn ring() -> Self {
        Self {
            notes: vec![(783.99, 0.0), (659.25, 0.16), (783.99, 0.5), (659.25, 0.66)],
            shape: Shape::Triangle,
            peak: 0.2,
            length: 0.32,
        }
    }

    pub fn seconds(&self) -> f32 {
        self.notes.iter().map(|(_, start)| start + self.length).fold(0.0, f32::max)
    }

    /// The tune as one channel of samples between -1 and 1.
    pub fn render(&self, rate: u32) -> Vec<f32> {
        const QUIET: f32 = 0.0001;
        const RISE: f32 = 0.02;
        let rate = rate as f32;
        let mut samples = vec![0.0f32; (self.seconds() * rate).ceil() as usize];
        for (pitch, start) in &self.notes {
            let first = (start * rate) as usize;
            let count = (self.length * rate) as usize;
            for (index, sample) in samples.iter_mut().skip(first).take(count).enumerate() {
                let time = index as f32 / rate;
                // The loudness climbs fast and then falls away, both along a curve, not a line.
                let gain = if time < RISE {
                    QUIET * (self.peak / QUIET).powf(time / RISE)
                } else {
                    self.peak * (QUIET / self.peak).powf((time - RISE) / (self.length - RISE))
                };
                let turn = std::f32::consts::TAU * pitch * time;
                let wave = match self.shape {
                    Shape::Sine => turn.sin(),
                    Shape::Triangle => turn.sin().asin() * std::f32::consts::FRAC_2_PI,
                };
                *sample += wave * gain;
            }
        }
        samples
    }
}

/// Plays a tune on the default speakers and returns at once. A computer without speakers stays
/// quiet.
pub fn play(tune: Tune) {
    let _ = std::thread::Builder::new().name("telinha-tone".into()).spawn(move || {
        let _ = play_with(|rate| tune.render(rate));
    });
}

/// Plays samples made for the rate of the speakers, and waits for the end.
pub fn play_with(render: impl FnOnce(u32) -> Vec<f32>) -> Result<Duration, String> {
    let device = crate::audio::output_device(&cpal::default_host()).ok_or("nenhuma saída de som")?;
    let supported = device.default_output_config().map_err(|error| error.to_string())?;
    let channels = usize::from(supported.channels());
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate;

    let samples: Arc<[f32]> = render(rate).into();
    let length = Duration::from_secs_f32(samples.len() as f32 / rate as f32);
    let position = Arc::new(AtomicUsize::new(0));
    let next = {
        let samples = Arc::clone(&samples);
        move || samples.get(position.fetch_add(1, Ordering::Relaxed)).copied().unwrap_or(0.0)
    };
    let failed = |error: cpal::Error| eprintln!("telinha: toque: {error}");
    let stream = match format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                for frame in data.chunks_mut(channels) {
                    frame.fill(next());
                }
            },
            failed,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream(
            config,
            move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                for frame in data.chunks_mut(channels) {
                    frame.fill((next().clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16);
                }
            },
            failed,
            None,
        ),
        other => return Err(format!("formato de som sem suporte: {other:?}")),
    }
    .map_err(|error| error.to_string())?;
    stream.play().map_err(|error| error.to_string())?;
    // A little past the end, for what is still on its way to the speakers.
    std::thread::sleep(length + Duration::from_millis(150));
    Ok(length)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tune_lasts_until_its_last_note_fades_and_never_gets_too_loud() {
        let tune = Tune::join();
        assert!((tune.seconds() - 0.40).abs() < 0.001);
        let samples = tune.render(48_000);
        assert_eq!(samples.len(), 19_200);
        let loudest = samples.iter().fold(0.0f32, |loudest, sample| loudest.max(sample.abs()));
        assert!(loudest > 0.3 && loudest <= 0.6, "loudest was {loudest}");
        // It starts and ends in silence, so nothing clicks.
        assert!(samples[0].abs() < 0.001 && samples[samples.len() - 1].abs() < 0.001);

        for tune in [Tune::mic_off(), Tune::mic_on(), Tune::deaf_on(), Tune::deaf_off(), Tune::message()] {
            let samples = tune.render(48_000);
            assert!(samples.iter().all(|sample| sample.abs() <= 0.6));
            assert!(samples[0].abs() < 0.001 && samples[samples.len() - 1].abs() < 0.001);
        }
        assert_ne!(Tune::mic_off().notes, Tune::deaf_on().notes);

        let ring = Tune::ring();
        assert!((ring.seconds() - 0.98).abs() < 0.001);
        assert!(ring.render(44_100).iter().all(|sample| sample.abs() <= 0.41));
    }

    #[test]
    fn the_first_note_has_the_pitch_it_was_given() {
        // 440 Hz crosses zero going up 440 times a second.
        let tune = Tune { notes: vec![(440.0, 0.0)], shape: Shape::Sine, peak: 0.5, length: 1.0 };
        let samples = tune.render(48_000);
        let crossings = samples.windows(2).filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0).count();
        assert!((438..=442).contains(&crossings), "crossings were {crossings}");
    }

    /// Run by hand: `cargo test -p telinha-voice tones -- --ignored`. Opens the real speakers
    /// and plays silence through them, so it checks the path without making a sound.
    #[test]
    #[ignore]
    fn the_speakers_take_a_quarter_second_of_silence() {
        let played = play_with(|rate| vec![0.0; rate as usize / 4]).unwrap();
        assert!((played.as_secs_f32() - 0.25).abs() < 0.01);
    }
}
