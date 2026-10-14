//! Viewer side: decodes the host's packets and plays them on the default output device.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};

use crate::opus_codec::Decoder;
use crate::{CHANNELS, Resampler, SAMPLE_RATE};

/// Sound queued before playback starts (and again after running dry), in ms: absorbs
/// network jitter without much delay.
const PREBUFFER_MS: usize = 60;
/// More than this queued (the two clocks drift, or a burst arrived): drop the oldest.
const MAX_QUEUE_MS: usize = 250;
/// Lost packets concealed at most, before simply skipping ahead.
const MAX_CONCEALED: u16 = 5;

/// Interleaved stereo at the device rate, shared with the output callback.
#[derive(Default)]
struct Queue {
    samples: VecDeque<f32>,
    /// Waiting for `PREBUFFER_MS` before playing.
    buffering: bool,
}

struct Incoming {
    decoder: Decoder,
    resampler: Resampler,
    next_seq: Option<u16>,
    decoded: Vec<f32>,
    resampled: Vec<f32>,
}

/// Plays a host's sound until dropped.
pub struct Player {
    queue: Arc<Mutex<Queue>>,
    incoming: Mutex<Incoming>,
    /// Device rate, for the queue limits.
    rate: u32,
    /// Volume (0.0–1.0, as `f32` bits), applied by the output callback.
    volume: Arc<AtomicU32>,
    _stop: mpsc::Sender<()>,
}

impl Player {
    pub fn start() -> Result<Self> {
        let queue = Arc::new(Mutex::new(Queue {
            buffering: true,
            ..Queue::default()
        }));
        let (stop, stopped) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32>>();
        let shared = queue.clone();
        let volume = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let gain = volume.clone();
        // cpal streams are not `Send`: a thread owns the stream until the player is dropped.
        std::thread::Builder::new()
            .name("audio-playback".into())
            .spawn(move || match open(shared, gain) {
                Ok((stream, rate)) => {
                    let _ = ready_tx.send(Ok(rate));
                    let _ = stopped.recv();
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })?;
        let rate = ready_rx
            .recv()
            .map_err(|_| anyhow!("the playback thread ended"))??;
        Ok(Self {
            queue,
            incoming: Mutex::new(Incoming {
                decoder: Decoder::new()?,
                resampler: Resampler::new(SAMPLE_RATE, rate),
                next_seq: None,
                decoded: Vec::new(),
                resampled: Vec::new(),
            }),
            rate,
            volume,
            _stop: stop,
        })
    }

    /// Sets the volume, 0.0 (silent) to 1.0 (as sent).
    pub fn set_volume(&self, volume: f32) {
        self.volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Queues one wire packet (see [`crate::packet`]).
    pub fn push(&self, packet: &[u8]) {
        let Some((seq, opus)) = crate::parse(packet) else {
            return;
        };
        let mut incoming = self.incoming.lock().unwrap();
        let incoming = &mut *incoming;
        incoming.decoded.clear();
        if let Some(expected) = incoming.next_seq {
            let ahead = seq.wrapping_sub(expected);
            // Late (arrived after a newer one): useless now.
            if ahead > u16::MAX / 2 {
                return;
            }
            for _ in 0..ahead.min(MAX_CONCEALED) {
                let _ = incoming.decoder.decode(None, &mut incoming.decoded);
            }
        }
        incoming.next_seq = Some(seq.wrapping_add(1));
        if let Err(e) = incoming.decoder.decode(Some(opus), &mut incoming.decoded) {
            tracing::debug!("audio packet not decoded: {e:#}");
        }
        incoming.resampled.clear();
        incoming
            .resampler
            .process(&incoming.decoded, &mut incoming.resampled);

        let per_ms = self.rate as usize * CHANNELS / 1000;
        let mut queue = self.queue.lock().unwrap();
        queue.samples.extend(incoming.resampled.iter().copied());
        let excess = queue.samples.len().saturating_sub(MAX_QUEUE_MS * per_ms);
        if excess > 0 {
            // Keep about the prebuffer: latency must not creep up.
            let drop = excess + (MAX_QUEUE_MS - PREBUFFER_MS) * per_ms;
            let drop = drop.min(queue.samples.len()) & !1;
            queue.samples.drain(..drop);
        }
        if queue.buffering && queue.samples.len() >= PREBUFFER_MS * per_ms {
            queue.buffering = false;
        }
    }
}

/// Opens the default output device; returns the stream and its sample rate.
fn open(queue: Arc<Mutex<Queue>>, volume: Arc<AtomicU32>) -> Result<(Stream, u32)> {
    let device = cpal::default_host()
        .default_output_device()
        .context("no audio output device")?;
    let supported = device.default_output_config()?;
    let format = supported.sample_format();
    let config = supported.config();
    let (channels, rate) = (usize::from(config.channels), config.sample_rate);
    let on_error = |e| tracing::warn!("sound playback: {e}");
    // Fills `frames` of `channels` from the queue (silence while buffering or dry).
    let fill = move |out: &mut dyn FnMut(usize, f32), frames: usize| {
        let mut queue = queue.lock().unwrap();
        let gain = f32::from_bits(volume.load(Ordering::Relaxed));
        for frame in 0..frames {
            let stereo = if queue.buffering || queue.samples.len() < 2 {
                if !queue.buffering {
                    // Ran dry: wait for a little sound again before resuming.
                    queue.buffering = true;
                }
                [0.0, 0.0]
            } else {
                let l = queue.samples.pop_front().unwrap_or(0.0);
                let r = queue.samples.pop_front().unwrap_or(0.0);
                [l, r]
            };
            for c in 0..channels {
                let s = match (channels, c) {
                    (1, _) => (stereo[0] + stereo[1]) * 0.5,
                    (_, 0) => stereo[0],
                    (_, 1) => stereo[1],
                    _ => 0.0,
                };
                out(frame * channels + c, s * gain);
            }
        }
    };
    let stream = match format {
        SampleFormat::F32 => device.build_output_stream::<f32, _, _>(
            config,
            move |data, _| {
                let frames = data.len() / channels;
                fill(&mut |i, s| data[i] = s, frames);
            },
            on_error,
            None,
        )?,
        SampleFormat::I16 => device.build_output_stream::<i16, _, _>(
            config,
            move |data, _| {
                let frames = data.len() / channels;
                fill(
                    &mut |i, s| data[i] = (s.clamp(-1.0, 1.0) * 32_767.0) as i16,
                    frames,
                );
            },
            on_error,
            None,
        )?,
        other => anyhow::bail!("unsupported sound format {other:?}"),
    };
    stream.play()?;
    tracing::info!(channels, rate, ?format, "sound playback started");
    Ok((stream, rate))
}
