//! Audio as the recorder delivers it (raw 16 kHz mono signed 16-bit little-endian PCM) and
//! as the engines read it (a WAV file): the WAV bytes, loudness for the level meter, and
//! the silence gate.

/// Samples per second, what whisper.cpp and parakeet expect.
pub const SAMPLE_RATE: u32 = 16_000;
/// The silence gate's window: 50 ms.
const WINDOW: usize = SAMPLE_RATE as usize / 20;
/// The level meter's floor: quieter than this reads as 0.
const METER_FLOOR_DB: f32 = -60.0;

/// Samples from raw little-endian PCM bytes; an odd last byte (a short read) is ignored.
pub fn samples(bytes: &[u8]) -> Vec<i16> {
    bytes.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect()
}

/// A complete WAV file (RIFF, PCM, 16 kHz, mono, 16-bit) holding `samples`.
pub fn wav(samples: &[i16]) -> Vec<u8> {
    const CHANNELS: u16 = 1;
    const BITS: u16 = 16;
    let block = CHANNELS * BITS / 8;
    // Clips are capped at `max_seconds` (600 s is about 19 MB), far below u32::MAX.
    let data = u32::try_from(samples.len() * 2).unwrap_or(u32::MAX - 36);
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&CHANNELS.to_le_bytes());
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * u32::from(block)).to_le_bytes());
    out.extend_from_slice(&block.to_le_bytes());
    out.extend_from_slice(&BITS.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Root mean square of `samples`, 0 (silence or empty) to 1 (full scale).
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "a sample count far below 2^52, and a result from 0 to 1"
)]
pub fn rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    ((sum / samples.len() as f64).sqrt() / 32768.0) as f32
}

/// The loudest 50 ms window's RMS: one spoken word in a long quiet clip still counts.
pub fn peak_rms(samples: &[i16]) -> f32 {
    samples.chunks(WINDOW).map(rms).fold(0.0, f32::max)
}

/// The silence gate: nothing in the clip is louder than `threshold` (RMS, 0 to 1). Whisper
/// makes up text ("Thank you.") for silent clips, so these are not transcribed.
pub fn is_silent(samples: &[i16], threshold: f32) -> bool {
    peak_rms(samples) < threshold
}

/// `rms` on the meter's 0 to 1 scale: -60 dBFS and below is 0, full scale is 1.
pub fn meter(rms: f32) -> f32 {
    if rms <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * rms.log10();
    ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn reads_little_endian_pcm() {
        assert_eq!(samples(&[0x01, 0x00, 0xff, 0xff, 0x00, 0x80, 0x7f]), [1, -1, i16::MIN]);
        assert!(samples(&[]).is_empty());
    }

    #[test]
    fn writes_a_16k_mono_16_bit_wav() {
        let bytes = wav(&[1, -2]);
        assert_eq!(bytes.len(), 48);
        let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        let u16_at = |i: usize| u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap());
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(u32_at(4), 40);
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!((u32_at(16), u16_at(20), u16_at(22)), (16, 1, 1));
        assert_eq!((u32_at(24), u32_at(28)), (16_000, 32_000));
        assert_eq!((u16_at(32), u16_at(34)), (2, 16));
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(u32_at(40), 4);
        assert_eq!(&bytes[44..], [1, 0, 0xfe, 0xff]);
        assert_eq!(wav(&[]).len(), 44);
    }

    #[test]
    fn measures_loudness() {
        assert!(close(rms(&[]), 0.0));
        assert!(close(rms(&[0; 100]), 0.0));
        assert!(close(rms(&[i16::MIN; 10]), 1.0));
        assert!(close(rms(&[16384, -16384]), 0.5));
        assert!(close(meter(0.0), 0.0));
        assert!(close(meter(1.0), 1.0));
        assert!(close(meter(0.001), 0.0));
        assert!(close(meter(0.0001), 0.0));
        assert!(close(meter(0.031_623), 0.5));
    }

    #[test]
    fn the_gate_hears_one_loud_window_in_a_quiet_clip() {
        let quiet = vec![10i16; SAMPLE_RATE as usize * 5];
        assert!(is_silent(&quiet, 0.01));
        assert!(is_silent(&[], 0.01));
        let mut word = quiet.clone();
        word[40_000..40_800].fill(3000);
        assert!(close(peak_rms(&word), 3000.0 / 32768.0));
        assert!(!is_silent(&word, 0.01));
        // Averaged over the whole clip the word would be below the threshold.
        assert!(rms(&word) < 0.01);
        assert!(!is_silent(&quiet, 0.0));
    }
}
