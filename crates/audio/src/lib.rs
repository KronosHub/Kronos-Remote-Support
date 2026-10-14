//! Sound of the shared computer, sent to the viewer (ADR 0044).
//!
//! - Host: [`Capture`] records what the computer plays (macOS: ScreenCaptureKit; Windows:
//!   WASAPI loopback) and hands out Opus packets of 20 ms, 48 kHz stereo.
//! - Viewer: [`Player`] decodes the packets and plays them with a small jitter buffer.
//!
//! Packets travel on an unordered, unreliable data channel: a late packet is worth nothing,
//! a lost one is concealed by the decoder. Linux is not supported (both sides report it).

#[cfg(any(target_os = "macos", windows))]
mod opus_codec;
#[cfg(any(target_os = "macos", windows))]
mod playback;
mod resample;
#[cfg(any(target_os = "macos", windows))]
mod source;

pub use resample::Resampler;

/// Opus runs at 48 kHz; every packet carries 20 ms of stereo sound.
pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
pub const FRAME_SAMPLES: usize = 960;
/// Encoded bitrate: transparent for music, small next to the video.
pub const BITRATE: i32 = 64_000;

/// Packet on the wire: 2-byte big-endian sequence number, then the Opus payload.
pub fn packet(seq: u16, opus: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + opus.len());
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(opus);
    out
}

/// Splits a wire packet into sequence number and Opus payload.
pub fn parse(packet: &[u8]) -> Option<(u16, &[u8])> {
    let (seq, opus) = packet.split_first_chunk::<2>()?;
    Some((u16::from_be_bytes(*seq), opus))
}

/// This machine can record its own sound for a viewer.
pub const fn capture_supported() -> bool {
    cfg!(any(target_os = "macos", windows))
}

/// This machine can play a host's sound.
pub const fn playback_supported() -> bool {
    cfg!(any(target_os = "macos", windows))
}

#[cfg(any(target_os = "macos", windows))]
pub use imp::{Capture, Player};

#[cfg(any(target_os = "macos", windows))]
mod imp {
    use std::sync::mpsc;

    use anyhow::Result;

    use crate::opus_codec::Encoder;
    use crate::{CHANNELS, FRAME_SAMPLES, Resampler, SAMPLE_RATE};

    /// Records this computer's sound until dropped.
    pub struct Capture {
        _source: crate::source::Source,
    }

    impl Capture {
        /// Starts recording; `on_packet` receives each wire packet (see [`crate::packet`])
        /// on a worker thread.
        pub fn start(mut on_packet: impl FnMut(Vec<u8>) + Send + 'static) -> Result<Self> {
            let (pcm_tx, pcm_rx) = mpsc::sync_channel::<(Vec<f32>, u32)>(64);
            let source = crate::source::Source::start(pcm_tx)?;
            let mut encoder = Encoder::new()?;
            std::thread::Builder::new()
                .name("audio-encoder".into())
                .spawn(move || {
                    let mut resampler: Option<(u32, Resampler)> = None;
                    let mut pending: Vec<f32> = Vec::with_capacity(FRAME_SAMPLES * CHANNELS * 4);
                    let mut resampled = Vec::new();
                    let mut seq = 0u16;
                    // Ends when the source is dropped (its sender goes away).
                    while let Ok((pcm, rate)) = pcm_rx.recv() {
                        let resampler = match &mut resampler {
                            Some((r, resampler)) if *r == rate => resampler,
                            slot => &mut slot.insert((rate, Resampler::new(rate, SAMPLE_RATE))).1,
                        };
                        resampled.clear();
                        resampler.process(&pcm, &mut resampled);
                        pending.extend_from_slice(&resampled);
                        let frame = FRAME_SAMPLES * CHANNELS;
                        while pending.len() >= frame {
                            match encoder.encode(&pending[..frame]) {
                                Ok(opus) => {
                                    on_packet(crate::packet(seq, &opus));
                                    seq = seq.wrapping_add(1);
                                }
                                Err(e) => tracing::warn!("audio encoding failed: {e:#}"),
                            }
                            pending.drain(..frame);
                        }
                    }
                })?;
            Ok(Self { _source: source })
        }
    }

    pub use crate::playback::Player;
}
