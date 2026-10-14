//! Opus encoder and decoder at 48 kHz stereo, 20 ms frames.

use anyhow::{Context, Result};
use opus::{Application, Bitrate, Channels};

use crate::{BITRATE, CHANNELS, FRAME_SAMPLES, SAMPLE_RATE};

pub struct Encoder {
    inner: opus::Encoder,
    buf: Vec<u8>,
}

impl Encoder {
    pub fn new() -> Result<Self> {
        let mut inner = opus::Encoder::new(SAMPLE_RATE, Channels::Stereo, Application::Audio)
            .context("creating the Opus encoder")?;
        inner.set_bitrate(Bitrate::Bits(BITRATE))?;
        Ok(Self {
            inner,
            buf: vec![0; 4000],
        })
    }

    /// Encodes one frame ([`FRAME_SAMPLES`] × [`CHANNELS`] interleaved samples).
    pub fn encode(&mut self, pcm: &[f32]) -> Result<Vec<u8>> {
        let len = self.inner.encode_float(pcm, &mut self.buf)?;
        Ok(self.buf[..len].to_vec())
    }
}

pub struct Decoder {
    inner: opus::Decoder,
}

impl Decoder {
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: opus::Decoder::new(SAMPLE_RATE, Channels::Stereo)
                .context("creating the Opus decoder")?,
        })
    }

    /// Decodes a packet, or conceals a lost one (`None`), into interleaved stereo.
    pub fn decode(&mut self, opus: Option<&[u8]>, out: &mut Vec<f32>) -> Result<()> {
        // Room for the longest Opus frame (120 ms); a concealed frame takes the buffer's
        // length, so that one gets exactly one frame.
        let len = if opus.is_some() { 6 } else { 1 } * FRAME_SAMPLES * CHANNELS;
        let mut pcm = vec![0f32; len];
        let samples = self
            .inner
            .decode_float(opus.unwrap_or(&[]), &mut pcm, false)?;
        out.extend_from_slice(&pcm[..samples * CHANNELS]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_survives_the_round_trip() {
        let mut encoder = Encoder::new().unwrap();
        let mut decoder = Decoder::new().unwrap();
        // 1 kHz tone on both channels.
        let pcm: Vec<f32> = (0..FRAME_SAMPLES)
            .flat_map(|i| {
                let s = (i as f32 * 2.0 * std::f32::consts::PI * 1000.0 / 48_000.0).sin() * 0.5;
                [s, s]
            })
            .collect();
        let mut out = Vec::new();
        for _ in 0..5 {
            let packet = encoder.encode(&pcm).unwrap();
            assert!(packet.len() < 400, "{} bytes", packet.len());
            decoder.decode(Some(&packet), &mut out).unwrap();
        }
        assert_eq!(out.len(), 5 * FRAME_SAMPLES * CHANNELS);
        let energy: f32 = out[out.len() / 2..].iter().map(|s| s * s).sum();
        assert!(energy > 10.0, "decoded sound is silent ({energy})");
        // A lost packet is concealed with a full frame.
        let before = out.len();
        decoder.decode(None, &mut out).unwrap();
        assert_eq!(out.len() - before, FRAME_SAMPLES * CHANNELS);
    }
}
