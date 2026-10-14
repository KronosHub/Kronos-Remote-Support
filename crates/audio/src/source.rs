//! What the computer plays, as interleaved stereo `f32` chunks with their sample rate.

use std::sync::mpsc::SyncSender;

/// Chunks of interleaved stereo and their sample rate, for the encoder thread.
pub type PcmSender = SyncSender<(Vec<f32>, u32)>;

/// Hands a chunk to the encoder; drops it if the encoder is behind (sound is live).
fn deliver(tx: &PcmSender, pcm: Vec<f32>, rate: u32) {
    let _ = tx.try_send((pcm, rate));
}

/// Converts `channels` interleaved channels to stereo (mono doubled, extra channels dropped).
fn to_stereo(samples: impl Iterator<Item = f32>, channels: usize, out: &mut Vec<f32>) {
    let mut frame = [0f32; 2];
    for (i, s) in samples.enumerate() {
        let c = i % channels.max(1);
        if c < 2 {
            frame[c] = s;
        }
        if c == channels.max(1) - 1 {
            if channels == 1 {
                frame[1] = frame[0];
            }
            out.extend_from_slice(&frame);
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac::Source;

#[cfg(target_os = "macos")]
mod mac {
    use anyhow::{Context, Result};
    use screencapturekit::prelude::*;

    use super::{PcmSender, deliver};

    /// `kAudioFormatFlagIsNonInterleaved`: one buffer per channel.
    const NON_INTERLEAVED: u32 = 1 << 5;

    /// A ScreenCaptureKit stream of its own, for sound only (tiny video, 1 fps, ignored).
    pub struct Source {
        stream: SCStream,
    }

    impl Source {
        pub fn start(tx: PcmSender) -> Result<Self> {
            let content = SCShareableContent::get().context("screen recording permission")?;
            let display = content
                .displays()
                .into_iter()
                .next()
                .context("no display to attach the sound capture to")?;
            let filter = SCContentFilter::create()
                .with_display(&display)
                .with_excluding_windows(&[])
                .build()?;
            let config = SCStreamConfiguration::new()
                .with_width(2)
                .with_height(2)
                .with_minimum_frame_interval(&CMTime::new(1, 1))
                .with_captures_audio(true)
                // Our own app makes no sound worth sending (and must not echo).
                .with_excludes_current_process_audio(true)
                .with_sample_rate(crate::SAMPLE_RATE as i32)
                .with_channel_count(crate::CHANNELS as i32);
            let mut stream = SCStream::new(&filter, &config)?;
            stream.add_output_handler(Ignore, SCStreamOutputType::Screen)?;
            stream.add_output_handler(Sound { tx }, SCStreamOutputType::Audio)?;
            stream
                .start_capture()
                .context("starting the sound capture")?;
            tracing::info!("sound capture started (ScreenCaptureKit)");
            Ok(Self { stream })
        }
    }

    impl Drop for Source {
        fn drop(&mut self) {
            let _ = self.stream.stop_capture();
        }
    }

    struct Ignore;

    impl SCStreamOutputTrait for Ignore {
        fn did_output_sample_buffer(&self, _sample: CMSampleBuffer, _kind: SCStreamOutputType) {}
    }

    struct Sound {
        tx: PcmSender,
    }

    impl SCStreamOutputTrait for Sound {
        fn did_output_sample_buffer(&self, sample: CMSampleBuffer, kind: SCStreamOutputType) {
            if !matches!(kind, SCStreamOutputType::Audio) {
                return;
            }
            let format = sample.format_description();
            let rate = format
                .as_ref()
                .and_then(|f| f.audio_sample_rate())
                .map_or(crate::SAMPLE_RATE, |r| r as u32);
            let planar = format
                .as_ref()
                .and_then(|f| f.audio_format_flags())
                .is_some_and(|flags| flags & NON_INTERLEAVED != 0);
            let Ok(list) = sample.audio_buffer_list() else {
                return;
            };
            let floats = |data: &[u8]| -> Vec<f32> {
                data.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_ne_bytes(*b))
                    .collect()
            };
            let mut pcm = Vec::new();
            if planar && list.num_buffers() >= 2 {
                let (Some(left), Some(right)) = (list.get(0), list.get(1)) else {
                    return;
                };
                let (left, right) = (floats(left.data()), floats(right.data()));
                for (l, r) in left.iter().zip(&right) {
                    pcm.extend_from_slice(&[*l, *r]);
                }
            } else if let Some(buffer) = list.get(0) {
                let channels = buffer.number_channels.max(1) as usize;
                super::to_stereo(floats(buffer.data()).into_iter(), channels, &mut pcm);
            }
            if !pcm.is_empty() {
                deliver(&self.tx, pcm, rate);
            }
        }
    }
}

#[cfg(windows)]
pub use win::Source;

#[cfg(windows)]
mod win {
    use std::sync::mpsc;

    use anyhow::{Context, Result, anyhow};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{SampleFormat, Stream};

    use super::{PcmSender, deliver, to_stereo};

    /// WASAPI loopback of the default output device. The cpal stream is not `Send`, so a
    /// thread owns it until the source is dropped.
    pub struct Source {
        _stop: mpsc::Sender<()>,
    }

    impl Source {
        pub fn start(tx: PcmSender) -> Result<Self> {
            let (stop, stopped) = mpsc::channel::<()>();
            let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
            std::thread::Builder::new()
                .name("audio-loopback".into())
                .spawn(move || match open(tx) {
                    Ok(stream) => {
                        let _ = ready_tx.send(Ok(()));
                        let _ = stopped.recv();
                        drop(stream);
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                })?;
            ready_rx
                .recv()
                .map_err(|_| anyhow!("the sound capture thread ended"))??;
            Ok(Self { _stop: stop })
        }
    }

    fn open(tx: PcmSender) -> Result<Stream> {
        let device = cpal::default_host()
            .default_output_device()
            .context("no audio output device")?;
        // An output device used for input records what it plays (loopback).
        let supported = device.default_output_config()?;
        let format = supported.sample_format();
        let config = supported.config();
        let (channels, rate) = (usize::from(config.channels), config.sample_rate);
        let on_error = |e| tracing::warn!("sound capture: {e}");
        let stream = match format {
            SampleFormat::F32 => device.build_input_stream::<f32, _, _>(
                config,
                move |data, _| {
                    let mut pcm = Vec::with_capacity(data.len() / channels * 2);
                    to_stereo(data.iter().copied(), channels, &mut pcm);
                    deliver(&tx, pcm, rate);
                },
                on_error,
                None,
            )?,
            SampleFormat::I16 => device.build_input_stream::<i16, _, _>(
                config,
                move |data, _| {
                    let mut pcm = Vec::with_capacity(data.len() / channels * 2);
                    let samples = data.iter().map(|&s| f32::from(s) / 32_768.0);
                    to_stereo(samples, channels, &mut pcm);
                    deliver(&tx, pcm, rate);
                },
                on_error,
                None,
            )?,
            other => anyhow::bail!("unsupported sound format {other:?}"),
        };
        stream.play()?;
        tracing::info!(
            channels,
            rate,
            ?format,
            "sound capture started (WASAPI loopback)"
        );
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::to_stereo;

    #[test]
    fn channels_become_stereo() {
        let mut out = Vec::new();
        to_stereo([1.0, 2.0].into_iter(), 1, &mut out);
        assert_eq!(out, [1.0, 1.0, 2.0, 2.0]);
        out.clear();
        to_stereo([1.0, 2.0, 3.0, 4.0, 5.0, 6.0].into_iter(), 3, &mut out);
        assert_eq!(out, [1.0, 2.0, 4.0, 5.0]);
    }
}
