//! Video encoding for the remote desktop stream.
//!
//! [`new_encoder`] picks the platform's hardware H.264 encoder through a minimal FFmpeg
//! (ADR 0058: VideoToolbox on macOS) when the app was built with it, and OpenH264 (software,
//! built from source) otherwise or when the hardware cannot be used.

#[cfg(kronos_hw)]
mod hardware;

use openh264::OpenH264API;
use openh264::Timestamp;
use openh264::encoder::{
    BitRate, Encoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod, QpRange,
    RateControlMode, UsageType,
};
use openh264::formats::{BgraSliceU8, YUVBuffer};
use openh264_sys2::{
    ENCODER_OPTION_BITRATE, ENCODER_OPTION_FRAME_RATE, SBitrateInfo, SPATIAL_LAYER_ALL,
};

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("frame size {got:?} does not match encoder size {expected:?}")]
    SizeMismatch {
        expected: (u32, u32),
        got: (u32, u32),
    },
    #[error("frame dimensions must be even and non-zero, got {0}x{1}")]
    InvalidSize(u32, u32),
    #[error("{0}x{1} exceeds the H.264 encoder limit of {MAX_WIDTH}x{MAX_HEIGHT}")]
    TooLarge(u32, u32),
    #[error("openh264: {0}")]
    OpenH264(#[from] openh264::Error),
    #[error("hardware encoder: {0}")]
    Hardware(String),
}

pub type Result<T> = std::result::Result<T, CodecError>;

/// Largest landscape frame OpenH264 accepts (portrait: swapped). Capture must scale to fit.
pub const MAX_WIDTH: u32 = 3840;
pub const MAX_HEIGHT: u32 = 2160;

/// Lowest quantizer allowed; see the rate control note in [`H264Encoder::new`].
const MIN_QP: u8 = 10;

fn fits_encoder(w: u32, h: u32) -> bool {
    (w <= MAX_WIDTH && h <= MAX_HEIGHT) || (w <= MAX_HEIGHT && h <= MAX_WIDTH)
}

#[derive(Debug, Clone)]
pub struct EncoderSettings {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_bps: u32,
    /// Frames between forced keyframes; 0 lets the encoder decide.
    pub keyframe_interval: u32,
}

impl EncoderSettings {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            fps: 30,
            bitrate_bps: 4_000_000,
            keyframe_interval: 0,
        }
    }
}

/// One encoded access unit in Annex B format (start-code delimited NAL units).
#[derive(Debug, Clone)]
pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub keyframe: bool,
}

/// What the stream needs from an H.264 encoder.
pub trait VideoEncoder {
    fn size(&self) -> (u32, u32);
    /// Next encoded frame will be an IDR.
    fn force_keyframe(&mut self);
    /// New target bitrate and frame rate, from the next frame (capped at the initial bitrate).
    fn set_rate(&mut self, bitrate_bps: u32, fps: u32);
    /// Encodes a tightly packed BGRA frame of the encoder's size; `None`: no output.
    fn encode_bgra(
        &mut self,
        bgra: &[u8],
        width: u32,
        height: u32,
        timestamp_ms: u64,
    ) -> Result<Option<EncodedFrame>>;
    /// Encodes the capture's own image without copying it (macOS: a `CVPixelBufferRef` of
    /// the encoder's size). `None` when this encoder cannot: pass BGRA instead.
    fn encode_native(
        &mut self,
        _image: *mut std::ffi::c_void,
        _timestamp_ms: u64,
    ) -> Option<Result<Option<EncodedFrame>>> {
        None
    }
    /// Short name for the logs ("VideoToolbox", "OpenH264").
    fn name(&self) -> &'static str;
    fn is_hardware(&self) -> bool {
        false
    }
}

/// Whether this build can encode in hardware at all (the machine may still refuse).
pub const HARDWARE_BUILT: bool = cfg!(kronos_hw);

/// The hardware encoder if `hardware` and it opens, else OpenH264. The error of a failed
/// hardware attempt comes back too, for the logs.
pub fn new_encoder(
    settings: &EncoderSettings,
    hardware: bool,
) -> Result<(Box<dyn VideoEncoder>, Option<CodecError>)> {
    #[cfg(kronos_hw)]
    let failure = if hardware {
        match hardware::HwEncoder::new(settings) {
            Ok(encoder) => return Ok((Box::new(encoder), None)),
            Err(e) => Some(e),
        }
    } else {
        None
    };
    #[cfg(not(kronos_hw))]
    let failure = {
        let _ = hardware;
        None
    };
    Ok((Box::new(H264Encoder::new(settings)?), failure))
}

pub struct H264Encoder {
    encoder: Encoder,
    yuv: YUVBuffer,
    size: (u32, u32),
    /// Upper bound for [`H264Encoder::set_bitrate`]: the bitrate the encoder was created with.
    max_bitrate: u32,
    /// OpenH264 initializes lazily on the first frame; options can only be set after that.
    initialized: bool,
    pending_rate: Option<(u32, u32)>,
}

impl H264Encoder {
    pub fn new(settings: &EncoderSettings) -> Result<Self> {
        let (w, h) = (settings.width, settings.height);
        if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 {
            return Err(CodecError::InvalidSize(w, h));
        }
        if !fits_encoder(w, h) {
            return Err(CodecError::TooLarge(w, h));
        }

        let config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .bitrate(BitRate::from_bps(settings.bitrate_bps))
            .max_frame_rate(FrameRate::from_hz(settings.fps as f32))
            .rate_control_mode(RateControlMode::Bitrate)
            .intra_frame_period(IntraFramePeriod::from_num_frames(
                settings.keyframe_interval,
            ))
            // Skipping frames under load froze the picture during motion; for remote
            // desktop a softer image beats a stalled one.
            .skip_frames(false)
            // Not supported for screen content; OpenH264 would warn and disable them anyway.
            .adaptive_quantization(false)
            .background_detection(false)
            // With a minimum QP of 0 OpenH264's rate control overshoots the target several
            // times over on busy screens (measured: 3.3 Mbit/s for a 1 Mbit/s target).
            .qp(QpRange::new(MIN_QP, 51));

        Ok(Self {
            encoder: Encoder::with_api_config(OpenH264API::from_source(), config)?,
            yuv: YUVBuffer::new(w as usize, h as usize),
            size: (w, h),
            max_bitrate: settings.bitrate_bps,
            initialized: false,
            pending_rate: None,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Next encoded frame will be an IDR (e.g. a new viewer joined or packets were lost).
    pub fn force_keyframe(&mut self) {
        self.encoder.force_intra_frame();
    }

    /// Changes the target bitrate and the frame rate the rate control plans for, on the fly
    /// (adaptive quality). The bitrate is capped at the one the encoder was created with.
    /// Takes effect from the next encoded frame. The caller must actually feed `fps` frames
    /// per second, otherwise each frame gets the wrong share of the bitrate.
    pub fn set_rate(&mut self, bitrate_bps: u32, fps: u32) {
        self.pending_rate = Some((bitrate_bps.clamp(1, self.max_bitrate), fps.max(1)));
    }

    fn apply_pending_rate(&mut self) -> Result<()> {
        let Some((bps, fps)) = self.pending_rate.take() else {
            return Ok(());
        };
        let mut info = SBitrateInfo {
            iLayer: SPATIAL_LAYER_ALL,
            iBitrate: bps as std::os::raw::c_int,
        };
        let mut frame_rate = fps as f32;
        // SAFETY: the encoder is initialized (a frame was encoded); both options take a
        // pointer to a value that only has to live for the call.
        let ok = unsafe {
            let raw = self.encoder.raw_api();
            raw.set_option(ENCODER_OPTION_FRAME_RATE, (&raw mut frame_rate).cast()) == 0
                && raw.set_option(ENCODER_OPTION_BITRATE, (&raw mut info).cast()) == 0
        };
        if !ok {
            return Err(CodecError::OpenH264(openh264::Error::msg(
                "cannot change the encoder rate",
            )));
        }
        Ok(())
    }

    /// Encodes a tightly packed BGRA frame. Returns `None` when the encoder skipped it
    /// (rate control decided to drop the frame).
    pub fn encode_bgra(
        &mut self,
        bgra: &[u8],
        width: u32,
        height: u32,
        timestamp_ms: u64,
    ) -> Result<Option<EncodedFrame>> {
        if (width, height) != self.size {
            return Err(CodecError::SizeMismatch {
                expected: self.size,
                got: (width, height),
            });
        }

        if self.initialized {
            self.apply_pending_rate()?;
        }
        self.yuv
            .read_bgra8(BgraSliceU8::new(bgra, (width as usize, height as usize)));
        let bitstream = self
            .encoder
            .encode_at(&self.yuv, Timestamp::from_millis(timestamp_ms))?;
        self.initialized = true;

        let frame_type = bitstream.frame_type();
        if matches!(frame_type, FrameType::Skip | FrameType::Invalid) {
            return Ok(None);
        }
        Ok(Some(EncodedFrame {
            data: bitstream.to_vec(),
            keyframe: matches!(frame_type, FrameType::IDR | FrameType::I),
        }))
    }
}

impl VideoEncoder for H264Encoder {
    fn size(&self) -> (u32, u32) {
        H264Encoder::size(self)
    }
    fn force_keyframe(&mut self) {
        H264Encoder::force_keyframe(self);
    }
    fn set_rate(&mut self, bitrate_bps: u32, fps: u32) {
        H264Encoder::set_rate(self, bitrate_bps, fps);
    }
    fn encode_bgra(
        &mut self,
        bgra: &[u8],
        width: u32,
        height: u32,
        timestamp_ms: u64,
    ) -> Result<Option<EncodedFrame>> {
        H264Encoder::encode_bgra(self, bgra, width, height, timestamp_ms)
    }
    fn name(&self) -> &'static str {
        "OpenH264"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_odd_size() {
        assert!(matches!(
            H264Encoder::new(&EncoderSettings::new(641, 480)),
            Err(CodecError::InvalidSize(641, 480))
        ));
    }

    #[test]
    fn rejects_too_large() {
        assert!(matches!(
            H264Encoder::new(&EncoderSettings::new(3456, 2234)),
            Err(CodecError::TooLarge(3456, 2234))
        ));
    }

    #[test]
    fn first_frame_is_keyframe_with_start_code() {
        let (w, h) = (320, 240);
        let mut enc = H264Encoder::new(&EncoderSettings::new(w, h)).unwrap();
        let bgra = vec![128u8; (w * h * 4) as usize];
        let frame = enc
            .encode_bgra(&bgra, w, h, 0)
            .unwrap()
            .expect("first frame encoded");
        assert!(frame.keyframe);
        assert!(frame.data.starts_with(&[0, 0, 0, 1]));
    }

    #[test]
    fn rejects_size_mismatch() {
        let mut enc = H264Encoder::new(&EncoderSettings::new(320, 240)).unwrap();
        let bgra = vec![0u8; 160 * 120 * 4];
        assert!(matches!(
            enc.encode_bgra(&bgra, 160, 120, 0),
            Err(CodecError::SizeMismatch { .. })
        ));
    }

    /// Average kbit/s of a busy screen (random "text" scrolling 7 px per frame plus a strip
    /// redrawn every frame) over 2 s, after 1 s to let rate control settle.
    fn busy_kbps(enc: &mut H264Encoder, fps: u32) -> usize {
        let (w, h) = enc.size();
        let mut seed = 7u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 24) as u8
        };
        let text: Vec<u8> = (0..(w / 4) * (h * 2)).map(|_| rnd()).collect();
        let mut bytes = 0;
        for n in 0..3 * fps {
            let strip: Vec<u8> = (0..(w / 4) * (h / 32 + 1)).map(|_| rnd()).collect();
            let bgra: Vec<u8> = (0..w * h)
                .flat_map(|i| {
                    let (x, y) = (i % w, i / w);
                    let v = if y < h / 8 {
                        strip[((y / 4) * (w / 4) + x / 4) as usize]
                    } else {
                        text[(((y + n * 7) % (h * 2)) / 4 * (w / 4) + x / 4) as usize]
                    };
                    [v, v, v, 255]
                })
                .collect();
            let ts = u64::from(n) * 1000 / u64::from(fps);
            let len = enc
                .encode_bgra(&bgra, w, h, ts)
                .unwrap()
                .map_or(0, |f| f.data.len());
            if n >= fps {
                bytes += len;
            }
        }
        bytes * 8 / 2000
    }

    #[test]
    fn rate_control_follows_target() {
        let mut enc = H264Encoder::new(&EncoderSettings {
            bitrate_bps: 1_000_000,
            ..EncoderSettings::new(640, 360)
        })
        .unwrap();
        let high = busy_kbps(&mut enc, 30);
        assert!((750..=1200).contains(&high), "{high} kbit/s for 1000");
        enc.set_rate(200_000, 10);
        let low = busy_kbps(&mut enc, 10);
        assert!((120..=280).contains(&low), "{low} kbit/s for 200");
        // Never above the initial bitrate.
        enc.set_rate(10_000_000, 30);
        let back = busy_kbps(&mut enc, 30);
        assert!(back <= 1200, "{back} kbit/s for 1000 (capped)");
    }
}
