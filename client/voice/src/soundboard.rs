//! The short sounds people fire in a call. A sound file is decoded once into what the call works
//! with, 48 kHz mono, and from there it is mixed into what the others hear and played here.

use std::sync::Arc;

use symphonia::core::audio::sample::Sample;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::audio::{RATE, Resampler};

/// A sound plays for at most this long. What is past it is left out.
pub const MAX_SECONDS: usize = 10;

/// Decodes an MP3, OGG Vorbis, WAV, FLAC or M4A file into 48 kHz mono. The error is a sentence
/// ready to show.
pub fn decode(data: &[u8]) -> Result<Vec<i16>, String> {
    let unreadable = || "não consegui ler esse som. Use MP3, OGG, WAV, FLAC ou M4A".to_owned();
    let source = MediaSourceStream::new(Box::new(std::io::Cursor::new(data.to_vec())), Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(&Hint::new(), source, FormatOptions::default(), MetadataOptions::default())
        .map_err(|_| unreadable())?;
    let track = format.default_track(TrackType::Audio).ok_or_else(unreadable)?;
    let params = track.codec_params.as_ref().and_then(|params| params.audio()).ok_or_else(unreadable)?;
    let rate = params.sample_rate.filter(|rate| *rate > 0).ok_or_else(unreadable)?;
    let mut decoder = symphonia::default::get_codecs().make_audio_decoder(params, &AudioDecoderOptions::default()).map_err(|_| unreadable())?;
    let track_id = track.id;

    let limit = MAX_SECONDS * rate as usize;
    let mut mono: Vec<f32> = Vec::new();
    let mut interleaved: Vec<f32> = Vec::new();
    while mono.len() < limit {
        let Ok(Some(packet)) = format.next_packet() else { break };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buffer) => {
                let frames = buffer.frames();
                if frames == 0 {
                    continue;
                }
                interleaved.resize(buffer.samples_interleaved(), f32::MID);
                buffer.copy_to_slice_interleaved(&mut interleaved);
                let channels = (interleaved.len() / frames).max(1);
                mono.extend(interleaved.chunks(channels).map(|frame| frame.iter().sum::<f32>() / channels as f32));
            }
            // A broken piece in the middle of a file is skipped, as players do.
            Err(Error::DecodeError(_)) => {}
            Err(_) => break,
        }
    }
    mono.truncate(limit);
    if mono.is_empty() {
        return Err(unreadable());
    }
    let mut at_call_rate = Vec::with_capacity(mono.len() * RATE as usize / rate as usize + 1);
    if rate == RATE {
        at_call_rate = mono;
    } else {
        Resampler::new(rate, RATE).push(&mono, &mut at_call_rate);
    }
    Ok(at_call_rate.iter().map(|sample| (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16).collect())
}

pub fn seconds(samples: &[i16]) -> f32 {
    samples.len() as f32 / RATE as f32
}

/// Plays a decoded sound on the speakers, outside of a call. Returns at once.
pub fn play(samples: Arc<[i16]>, volume: f32) {
    let _ = std::thread::Builder::new().name("telinha-sound".into()).spawn(move || {
        let _ = crate::tones::play_with(|rate| {
            let floats: Vec<f32> = samples.iter().map(|sample| f32::from(*sample) / f32::from(i16::MAX) * volume).collect();
            if rate == RATE {
                return floats;
            }
            let mut out = Vec::with_capacity(floats.len() * rate as usize / RATE as usize + 1);
            Resampler::new(RATE, rate).push(&floats, &mut out);
            out
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A WAV file with a steady note, written by hand: 16 bits, the rate and the channels given.
    fn wav(rate: u32, channels: u16, seconds: f32) -> Vec<u8> {
        let frames = (rate as f32 * seconds) as u32;
        let bytes = frames * u32::from(channels) * 2;
        let mut file = Vec::new();
        file.extend_from_slice(b"RIFF");
        file.extend_from_slice(&(36 + bytes).to_le_bytes());
        file.extend_from_slice(b"WAVEfmt ");
        file.extend_from_slice(&16u32.to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&channels.to_le_bytes());
        file.extend_from_slice(&rate.to_le_bytes());
        file.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        file.extend_from_slice(&(channels * 2).to_le_bytes());
        file.extend_from_slice(&16u16.to_le_bytes());
        file.extend_from_slice(b"data");
        file.extend_from_slice(&bytes.to_le_bytes());
        for frame in 0..frames {
            let sample = ((frame as f32 * std::f32::consts::TAU * 440.0 / rate as f32).sin() * 12000.0) as i16;
            for _ in 0..channels {
                file.extend_from_slice(&sample.to_le_bytes());
            }
        }
        file
    }

    #[test]
    fn a_sound_file_becomes_one_channel_at_the_rate_of_the_call() {
        let decoded = decode(&wav(44_100, 2, 0.5)).unwrap();
        assert!((seconds(&decoded) - 0.5).abs() < 0.01, "{} s", seconds(&decoded));
        let loudest = decoded.iter().map(|sample| sample.unsigned_abs()).max().unwrap();
        assert!((11_000..=12_100).contains(&loudest), "loudest {loudest}");
        // 440 Hz is still 440 Hz after the change of rate.
        let crossings = decoded.windows(2).filter(|pair| pair[0] <= 0 && pair[1] > 0).count();
        assert!((218..=222).contains(&crossings), "{crossings} crossings in half a second");

        let same_rate = decode(&wav(48_000, 1, 0.25)).unwrap();
        assert_eq!(same_rate.len(), 12_000);
    }

    #[test]
    fn a_long_sound_is_cut_and_what_is_not_sound_is_refused() {
        let long = decode(&wav(8_000, 1, 12.0)).unwrap();
        assert!((seconds(&long) - 10.0).abs() < 0.01, "{} s", seconds(&long));
        assert!(decode(b"this is not a sound").is_err());
        assert!(decode(&[]).is_err());
    }
}
