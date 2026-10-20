//! The hardware H.264 encoder behind `native/hwenc.c` (FFmpeg, ADR 0058).

use std::ffi::{c_char, c_int, c_void};

use crate::{CodecError, EncodedFrame, EncoderSettings, Result, VideoEncoder};

#[repr(C)]
struct KrPacket {
    data: *const u8,
    size: c_int,
    keyframe: c_int,
}

unsafe extern "C" {
    fn kr_hw_open(
        width: c_int,
        height: c_int,
        fps: c_int,
        bit_rate: i64,
        gop: c_int,
        err: *mut c_char,
        err_len: c_int,
    ) -> *mut c_void;
    fn kr_hw_set_rate(encoder: *mut c_void, bit_rate: i64, fps: c_int);
    fn kr_hw_encode_bgra(
        encoder: *mut c_void,
        bgra: *const u8,
        pts_ms: i64,
        keyframe: c_int,
        out: *mut KrPacket,
        err: *mut c_char,
        err_len: c_int,
    ) -> c_int;
    #[cfg(target_os = "macos")]
    fn kr_hw_encode_native(
        encoder: *mut c_void,
        image: *mut c_void,
        pts_ms: i64,
        keyframe: c_int,
        out: *mut KrPacket,
        err: *mut c_char,
        err_len: c_int,
    ) -> c_int;
    fn kr_hw_close(encoder: *mut c_void);
}

const ERR_LEN: usize = 256;

fn message(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

pub struct HwEncoder {
    raw: *mut c_void,
    size: (u32, u32),
    max_bitrate: u32,
    keyframe: bool,
}

impl HwEncoder {
    pub fn new(settings: &EncoderSettings) -> Result<Self> {
        let (w, h) = (settings.width, settings.height);
        if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 {
            return Err(CodecError::InvalidSize(w, h));
        }
        let mut err = [0 as c_char; ERR_LEN];
        // SAFETY: plain values in; the error buffer is ERR_LEN long.
        let raw = unsafe {
            kr_hw_open(
                w as c_int,
                h as c_int,
                settings.fps as c_int,
                i64::from(settings.bitrate_bps),
                settings.keyframe_interval as c_int,
                err.as_mut_ptr(),
                ERR_LEN as c_int,
            )
        };
        if raw.is_null() {
            return Err(CodecError::Hardware(message(&err)));
        }
        Ok(Self {
            raw,
            size: (w, h),
            max_bitrate: settings.bitrate_bps,
            // The first frame must be a keyframe for the viewer to start.
            keyframe: true,
        })
    }

    /// Runs one encode call and copies the packet out of the encoder's buffer.
    fn finish(
        &mut self,
        call: impl FnOnce(*mut KrPacket, *mut c_char) -> c_int,
    ) -> Result<Option<EncodedFrame>> {
        let mut packet = KrPacket {
            data: std::ptr::null(),
            size: 0,
            keyframe: 0,
        };
        let mut err = [0 as c_char; ERR_LEN];
        let status = call(&mut packet, err.as_mut_ptr());
        if status < 0 {
            return Err(CodecError::Hardware(message(&err)));
        }
        self.keyframe = false;
        if status == 0 || packet.data.is_null() || packet.size <= 0 {
            return Ok(None);
        }
        // SAFETY: the packet stays valid until the next call on this encoder.
        let data = unsafe { std::slice::from_raw_parts(packet.data, packet.size as usize) };
        Ok(Some(EncodedFrame {
            data: data.to_vec(),
            keyframe: packet.keyframe != 0,
        }))
    }
}

impl VideoEncoder for HwEncoder {
    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn force_keyframe(&mut self) {
        self.keyframe = true;
    }

    fn set_rate(&mut self, bitrate_bps: u32, fps: u32) {
        let bitrate = bitrate_bps.clamp(1, self.max_bitrate);
        // SAFETY: a live encoder.
        unsafe { kr_hw_set_rate(self.raw, i64::from(bitrate), fps.max(1) as c_int) };
    }

    fn encode_bgra(
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
        if bgra.len() < (width as usize) * (height as usize) * 4 {
            return Err(CodecError::InvalidSize(width, height));
        }
        let (raw, keyframe) = (self.raw, c_int::from(self.keyframe));
        self.finish(|packet, err| {
            // SAFETY: `bgra` holds a whole frame of the encoder's size.
            unsafe {
                kr_hw_encode_bgra(
                    raw,
                    bgra.as_ptr(),
                    timestamp_ms as i64,
                    keyframe,
                    packet,
                    err,
                    ERR_LEN as c_int,
                )
            }
        })
    }

    #[cfg(target_os = "macos")]
    fn encode_native(
        &mut self,
        image: *mut c_void,
        timestamp_ms: u64,
    ) -> Option<Result<Option<EncodedFrame>>> {
        let (raw, keyframe) = (self.raw, c_int::from(self.keyframe));
        Some(self.finish(|packet, err| {
            // SAFETY: `image` is a live CVPixelBufferRef of the encoder's size (caller).
            unsafe {
                kr_hw_encode_native(
                    raw,
                    image,
                    timestamp_ms as i64,
                    keyframe,
                    packet,
                    err,
                    ERR_LEN as c_int,
                )
            }
        }))
    }

    fn name(&self) -> &'static str {
        if cfg!(target_os = "macos") {
            "VideoToolbox"
        } else {
            "Media Foundation"
        }
    }

    fn is_hardware(&self) -> bool {
        true
    }
}

impl Drop for HwEncoder {
    fn drop(&mut self) {
        // SAFETY: opened by kr_hw_open, closed once.
        unsafe { kr_hw_close(self.raw) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grey screen with a moving bar, encoded by the media engine.
    #[test]
    fn encodes_frames_with_keyframe_first() {
        let (w, h) = (1280u32, 720u32);
        let mut enc = match HwEncoder::new(&EncoderSettings::new(w, h)) {
            Ok(enc) => enc,
            // CI machines may have no media engine.
            Err(e) => {
                eprintln!("no hardware encoder here: {e}");
                return;
            }
        };
        let mut keyframes = 0;
        let mut packets = 0;
        for n in 0..30u32 {
            let mut bgra = vec![90u8; (w * h * 4) as usize];
            for y in 0..h {
                for x in (n * 20)..(n * 20 + 40).min(w) {
                    let i = ((y * w + x) * 4) as usize;
                    bgra[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
            if let Some(frame) = enc.encode_bgra(&bgra, w, h, u64::from(n) * 33).unwrap() {
                assert!(frame.data.starts_with(&[0, 0, 0, 1]));
                if packets == 0 {
                    assert!(frame.keyframe, "first packet must be a keyframe");
                }
                packets += 1;
                keyframes += usize::from(frame.keyframe);
            }
        }
        // One packet per frame, no delay: the patched encoder waits for each frame (Media
        // Foundation answers asynchronously and may still be warming up on the first ones).
        if cfg!(target_os = "macos") {
            assert_eq!(packets, 30);
        } else {
            assert!(packets >= 25, "{packets} packets for 30 frames");
        }
        assert_eq!(keyframes, 1);
        enc.force_keyframe();
        let bgra = vec![10u8; (w * h * 4) as usize];
        let frame = enc.encode_bgra(&bgra, w, h, 2000).unwrap().unwrap();
        assert!(frame.keyframe);
    }
}
