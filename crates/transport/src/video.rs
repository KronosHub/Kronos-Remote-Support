//! H.264 over a WebRTC video track: sending encoded access units (host) and reassembling
//! them from RTP packets (viewer).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bytes::Bytes;
use rtc::media::Sample;
use rtc::peer_connection::configuration::media_engine::MIME_TYPE_H264;
use rtc::rtp::codec::h264::H264Packet;
use rtc::rtp::packetizer::Depacketizer;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
};
use rtc::rtp_transceiver::{PayloadType, SSRC};
use tokio::sync::OnceCell;
use webrtc::media_stream::MediaStreamTrack;
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::PeerConnection;
use webrtc::rtp_transceiver::RtpSender;

/// Constrained Baseline, packetization mode 1: what OpenH264 produces and every decoder takes.
const H264_FMTP: &str = "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f";

fn h264_codec() -> RTCRtpCodec {
    RTCRtpCodec {
        mime_type: MIME_TYPE_H264.to_owned(),
        clock_rate: 90_000,
        channels: 0,
        sdp_fmtp_line: H264_FMTP.to_owned(),
        rtcp_feedback: vec![],
    }
}

/// Host side: the outgoing screen track.
pub struct VideoSender {
    track: Arc<TrackLocalStaticSample>,
    sender: Arc<dyn RtpSender>,
    ssrc: SSRC,
    /// Payload type, known only once negotiation completed.
    payload_type: OnceCell<PayloadType>,
}

impl VideoSender {
    pub(crate) async fn attach(pc: &Arc<dyn PeerConnection>) -> Result<Self> {
        let ssrc = rand::random::<u32>();
        let track = Arc::new(TrackLocalStaticSample::new(
            Instant::now(),
            MediaStreamTrack::new(
                "screen".into(),
                "screen-video".into(),
                "screen".into(),
                RtpCodecKind::Video,
                vec![RTCRtpEncodingParameters {
                    rtp_coding_parameters: RTCRtpCodingParameters {
                        ssrc: Some(ssrc),
                        ..Default::default()
                    },
                    codec: h264_codec(),
                    ..Default::default()
                }],
            ),
        )?);
        let sender = pc.add_track(track.clone() as Arc<dyn TrackLocal>).await?;
        Ok(Self {
            track,
            sender,
            ssrc,
            payload_type: OnceCell::new(),
        })
    }

    async fn payload_type(&self) -> Result<PayloadType> {
        self.payload_type
            .get_or_try_init(|| async {
                self.sender
                    .get_parameters()
                    .await?
                    .rtp_parameters
                    .codecs
                    .first()
                    .map(|c| c.payload_type)
                    .context("video codec not negotiated")
            })
            .await
            .copied()
    }

    /// Sends one encoded access unit (Annex B). `duration` is the time since the previous frame.
    pub async fn write_frame(&self, data: Vec<u8>, duration: Duration) -> Result<()> {
        let payload_type = self.payload_type().await?;
        self.track
            .sample_writer(self.ssrc, payload_type)
            .write_sample(&Sample {
                data: Bytes::from(data),
                duration,
                ..Sample::new(Instant::now())
            })
            .await?;
        Ok(())
    }
}

/// One complete H.264 access unit in Annex B format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    pub data: Vec<u8>,
    pub keyframe: bool,
    /// RTP timestamp (90 kHz clock).
    pub rtp_timestamp: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Assembled {
    Frame(VideoFrame),
    /// Frames are being dropped until the next keyframe (packets lost, or waiting for the
    /// first keyframe): one should be requested. Repeats while the wait lasts.
    Lost,
}

/// How long a gap in sequence numbers may stay open before the missing packets are
/// declared lost. NACK retransmissions arrive out of order within about one RTT.
const MAX_REORDER_WAIT: Duration = Duration::from_millis(200);
/// Upper bound on packets held while waiting for a gap to fill.
const MAX_PENDING_PACKETS: usize = 1024;

struct Packet {
    timestamp: u32,
    marker: bool,
    payload: Bytes,
    arrived: Instant,
}

/// Rebuilds access units from RTP packets.
///
/// Packets go through a small reorder buffer (late retransmissions are normal with NACK);
/// only a gap that stays open longer than [`MAX_REORDER_WAIT`] counts as a loss, after which
/// frames are dropped until a keyframe resyncs the decoder.
pub struct FrameAssembler {
    depacketizer: H264Packet,
    buffer: Vec<u8>,
    timestamp: Option<u32>,
    /// Extended (wrap-free) sequence number expected next.
    next_seq: Option<u64>,
    last_ext: Option<u64>,
    pending: std::collections::BTreeMap<u64, Packet>,
    /// Set after a loss (and at start): only a keyframe can resync the decoder.
    need_keyframe: bool,
    stats: PacketStats,
}

/// Packet counters since the assembler started, for congestion feedback to the host.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PacketStats {
    pub received: u64,
    /// Packets given up on (never arrived, not even as a NACK retransmission).
    pub lost: u64,
}

impl Default for FrameAssembler {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameAssembler {
    pub fn new() -> Self {
        Self {
            depacketizer: H264Packet::default(),
            buffer: Vec::new(),
            timestamp: None,
            next_seq: None,
            last_ext: None,
            pending: Default::default(),
            need_keyframe: true,
            stats: PacketStats::default(),
        }
    }

    pub fn stats(&self) -> PacketStats {
        self.stats
    }

    /// Maps a 16-bit sequence number to a monotonically extended one, picking the value
    /// closest to the last seen.
    fn extend(&mut self, seq: u16) -> u64 {
        let ext = match self.last_ext {
            None => u64::from(seq) + (1 << 16),
            Some(last) => {
                let base = last & !0xffff;
                [base.wrapping_sub(1 << 16), base, base + (1 << 16)]
                    .into_iter()
                    .map(|b| b + u64::from(seq))
                    .min_by_key(|c| c.abs_diff(last))
                    .expect("three candidates")
            }
        };
        self.last_ext = Some(self.last_ext.map_or(ext, |l| l.max(ext)));
        ext
    }

    /// Feeds one RTP packet; returns the frames (and loss notices) it completes.
    pub fn push(
        &mut self,
        seq: u16,
        timestamp: u32,
        marker: bool,
        payload: &Bytes,
        now: Instant,
    ) -> Vec<Assembled> {
        let ext = self.extend(seq);
        let next = *self.next_seq.get_or_insert(ext);
        if ext < next || self.pending.contains_key(&ext) {
            return Vec::new(); // duplicate, or too late after the gap was given up
        }
        self.stats.received += 1;
        self.pending.insert(
            ext,
            Packet {
                timestamp,
                marker,
                payload: payload.clone(),
                arrived: now,
            },
        );

        let mut out = Vec::new();
        loop {
            self.drain_in_order(&mut out);
            // Waiting on a gap: the packet right after it is the oldest one held.
            let Some((&oldest, packet)) = self.pending.first_key_value() else {
                break;
            };
            // The wait counts from that packet's arrival, not from when this gap became the
            // first one: otherwise, under steady loss, each gap would wait its turn and the
            // picture would fall further and further behind.
            if now.duration_since(packet.arrived) < MAX_REORDER_WAIT
                && self.pending.len() < MAX_PENDING_PACKETS
            {
                break;
            }
            // Give up on the missing packets: skip to the oldest one we have.
            if let Some(next) = self.next_seq {
                self.stats.lost += oldest - next;
            }
            self.next_seq = Some(oldest);
            self.lost(&mut out);
        }
        out
    }

    fn drain_in_order(&mut self, out: &mut Vec<Assembled>) {
        while let Some(next) = self.next_seq {
            let Some(packet) = self.pending.remove(&next) else {
                break;
            };
            self.next_seq = Some(next + 1);
            self.process(packet, out);
        }
    }

    fn lost(&mut self, out: &mut Vec<Assembled>) {
        self.buffer.clear();
        self.timestamp = None;
        self.depacketizer = H264Packet::default();
        self.need_keyframe = true;
        out.push(Assembled::Lost);
    }

    fn process(&mut self, packet: Packet, out: &mut Vec<Assembled>) {
        // A new timestamp while a frame is half built: its last packet never came.
        if self.timestamp.is_some_and(|ts| ts != packet.timestamp) && !self.buffer.is_empty() {
            self.lost(out);
        }
        self.timestamp = Some(packet.timestamp);
        match self.depacketizer.depacketize(&packet.payload) {
            Ok(nals) => self.buffer.extend_from_slice(&nals),
            Err(_) => {
                self.lost(out);
                return;
            }
        }
        if !packet.marker {
            return;
        }
        let data = std::mem::take(&mut self.buffer);
        self.timestamp = None;
        if data.is_empty() {
            return;
        }
        let keyframe = is_keyframe(&data);
        if self.need_keyframe && !keyframe {
            // Still useless to the decoder: keep asking for a keyframe.
            out.push(Assembled::Lost);
            return;
        }
        self.need_keyframe = false;
        out.push(Assembled::Frame(VideoFrame {
            data,
            keyframe,
            rtp_timestamp: packet.timestamp,
        }));
    }
}

/// True if the Annex B access unit contains an IDR slice (NAL type 5).
pub fn is_keyframe(annex_b: &[u8]) -> bool {
    nal_types(annex_b).any(|t| t == 5)
}

fn nal_types(annex_b: &[u8]) -> impl Iterator<Item = u8> + '_ {
    annex_b
        .windows(4)
        .enumerate()
        .filter(|(_, w)| w[..3] == [0, 0, 1])
        .map(|(i, _)| i + 3)
        .filter_map(move |i| annex_b.get(i).map(|b| b & 0x1f))
}

/// Viewer side: incoming screen track.
pub struct VideoReceiver {
    track: Arc<dyn TrackRemote>,
    assembler: FrameAssembler,
    ready: std::collections::VecDeque<Assembled>,
    stats: Arc<SharedPacketStats>,
    test_loss: Option<TestLoss>,
}

/// Test hook: `KRS_TEST_PACKET_LOSS=<percent>:<seconds>` drops that share of the incoming
/// video packets for that long (from the first video packet), to exercise loss recovery
/// and adaptive quality without a lossy network. Only affects the process it is set for.
struct TestLoss {
    percent: u32,
    until: Instant,
    seed: u32,
}

impl TestLoss {
    fn from_env() -> Option<Self> {
        let value = std::env::var("KRS_TEST_PACKET_LOSS").ok()?;
        let (percent, seconds) = value.split_once(':')?;
        let loss = Self {
            percent: percent.parse().ok()?,
            until: Instant::now() + Duration::from_secs(seconds.parse().ok()?),
            seed: 1,
        };
        tracing::warn!("simulating {}% video packet loss", loss.percent);
        Some(loss)
    }

    fn drop_packet(&mut self) -> bool {
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        Instant::now() < self.until && (self.seed >> 8) % 100 < self.percent
    }
}

/// [`PacketStats`] readable from another task while the receiver runs.
#[derive(Debug, Default)]
pub struct SharedPacketStats {
    received: AtomicU64,
    lost: AtomicU64,
}

impl SharedPacketStats {
    pub fn get(&self) -> PacketStats {
        PacketStats {
            received: self.received.load(Ordering::Relaxed),
            lost: self.lost.load(Ordering::Relaxed),
        }
    }
}

impl VideoReceiver {
    pub fn new(track: Arc<dyn TrackRemote>) -> Self {
        Self::with_stats(track, Default::default())
    }

    /// Like [`VideoReceiver::new`], publishing packet counters to `stats`.
    pub fn with_stats(track: Arc<dyn TrackRemote>, stats: Arc<SharedPacketStats>) -> Self {
        Self {
            track,
            assembler: FrameAssembler::new(),
            ready: Default::default(),
            stats,
            test_loss: TestLoss::from_env(),
        }
    }

    /// Next frame or loss notice; `None` when the track ended.
    pub async fn next(&mut self) -> Option<Assembled> {
        loop {
            if let Some(item) = self.ready.pop_front() {
                return Some(item);
            }
            match self.track.poll().await? {
                TrackRemoteEvent::OnRtpPacket(packet) => {
                    if self.test_loss.as_mut().is_some_and(TestLoss::drop_packet) {
                        continue;
                    }
                    let h = &packet.header;
                    self.ready.extend(self.assembler.push(
                        h.sequence_number,
                        h.timestamp,
                        h.marker,
                        &packet.payload,
                        Instant::now(),
                    ));
                    let stats = self.assembler.stats();
                    self.stats.received.store(stats.received, Ordering::Relaxed);
                    self.stats.lost.store(stats.lost, Ordering::Relaxed);
                }
                TrackRemoteEvent::OnEnded => return None,
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDR: &[u8] = &[0x65, 0x88, 0x84];
    const SPS: &[u8] = &[0x67, 0x42, 0xc0, 0x1f];
    const P_SLICE: &[u8] = &[0x41, 0x9a, 0x02];

    fn single(nal: &[u8]) -> Bytes {
        Bytes::copy_from_slice(nal)
    }

    struct Feed {
        a: FrameAssembler,
        t0: Instant,
    }

    impl Feed {
        fn new() -> Self {
            Self {
                a: FrameAssembler::new(),
                t0: Instant::now(),
            }
        }
        fn push(&mut self, ms: u64, seq: u16, ts: u32, marker: bool, nal: &[u8]) -> Vec<Assembled> {
            self.a.push(
                seq,
                ts,
                marker,
                &single(nal),
                self.t0 + Duration::from_millis(ms),
            )
        }
    }

    fn frames(out: &[Assembled]) -> Vec<bool> {
        out.iter()
            .filter_map(|a| match a {
                Assembled::Frame(f) => Some(f.keyframe),
                Assembled::Lost => None,
            })
            .collect()
    }

    #[test]
    fn keyframe_detection() {
        assert!(is_keyframe(&[0, 0, 0, 1, 0x67, 1, 0, 0, 0, 1, 0x65, 2]));
        assert!(!is_keyframe(&[0, 0, 0, 1, 0x41, 1]));
    }

    #[test]
    fn waits_for_keyframe_then_passes_frames() {
        let mut f = Feed::new();
        // A P-frame before any keyframe is useless: ask for a keyframe.
        assert_eq!(f.push(0, 1, 100, true, P_SLICE), vec![Assembled::Lost]);
        assert!(f.push(10, 2, 200, false, SPS).is_empty());
        let out = f.push(10, 3, 200, true, IDR);
        let Some(Assembled::Frame(frame)) = out.first() else {
            panic!("expected keyframe, got {out:?}");
        };
        assert!(frame.keyframe);
        assert_eq!(
            frame.data,
            [&[0, 0, 0, 1][..], SPS, &[0, 0, 0, 1], IDR].concat()
        );
        assert_eq!(frames(&f.push(40, 4, 300, true, P_SLICE)), vec![false]);
    }

    #[test]
    fn reordered_packets_are_not_a_loss() {
        let mut f = Feed::new();
        f.push(0, 1, 100, true, IDR);
        // Packet 2 (frame ts 200) arrives after packet 3 (frame ts 300), e.g. a NACK resend.
        assert!(f.push(40, 3, 300, true, P_SLICE).is_empty());
        assert_eq!(
            frames(&f.push(60, 2, 200, true, P_SLICE)),
            vec![false, false]
        );
    }

    #[test]
    fn gap_open_too_long_is_a_loss() {
        let mut f = Feed::new();
        f.push(0, 1, 100, true, IDR);
        assert!(f.push(40, 3, 300, true, P_SLICE).is_empty());
        // 250 ms later packet 2 is still missing: loss; frames dropped until a keyframe.
        let out = f.push(290, 4, 400, true, P_SLICE);
        assert_eq!(out.first(), Some(&Assembled::Lost));
        assert!(frames(&out).is_empty());
        assert_eq!(frames(&f.push(300, 5, 500, true, IDR)), vec![true]);
        // The packet that finally shows up is ignored.
        assert!(f.push(310, 2, 200, true, P_SLICE).is_empty());
        assert_eq!(
            f.a.stats(),
            PacketStats {
                received: 4,
                lost: 1
            }
        );
    }

    #[test]
    fn steady_loss_does_not_build_latency() {
        let mut f = Feed::new();
        f.push(0, 1, 100, true, IDR);
        // One packet in four lost, one packet every 10 ms. Each gap must be given up 200 ms
        // after the packet following it arrived, never later.
        let mut last_frame_at = 0;
        let mut seq = 2u16;
        for ms in (10..3000).step_by(10) {
            if seq.is_multiple_of(4) {
                seq += 1; // lost
            }
            let nal = if seq % 20 == 1 { IDR } else { P_SLICE };
            let out = f.push(ms, seq, u32::from(seq) * 100, true, nal);
            if out.iter().any(|a| matches!(a, Assembled::Frame(_))) {
                last_frame_at = ms;
            }
            seq += 1;
        }
        assert!(
            last_frame_at >= 2700,
            "frames stalled at {last_frame_at} ms"
        );
        let stats = f.a.stats();
        assert!((85..=100).contains(&stats.lost), "{stats:?}");
        assert!(f.a.pending.len() < 30, "{} packets held", f.a.pending.len());
    }

    #[test]
    fn missing_marker_packet_is_detected() {
        let mut f = Feed::new();
        f.push(0, 1, 100, true, IDR);
        assert!(f.push(10, 2, 200, false, P_SLICE).is_empty());
        // Contiguous sequence, but the frame at ts 200 never ended.
        assert_eq!(
            f.push(40, 3, 300, true, P_SLICE).first(),
            Some(&Assembled::Lost)
        );
    }

    #[test]
    fn sequence_wraps_around() {
        let mut f = Feed::new();
        f.push(0, u16::MAX, 100, true, IDR);
        assert_eq!(frames(&f.push(30, 0, 200, true, P_SLICE)), vec![false]);
    }

    #[test]
    fn duplicates_are_ignored() {
        let mut f = Feed::new();
        f.push(0, 1, 100, true, IDR);
        assert!(f.push(5, 1, 100, true, IDR).is_empty());
    }
}
