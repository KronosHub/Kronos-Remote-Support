//! NACK retransmissions on a slow, lossy link (ADR 0076): with the library's defaults the
//! viewer asks again for each missing packet every 100 ms, with no limit, so at 1 s of
//! round trip every loss is resent about ten times. The duplicates fill the link (the "↓"
//! of the bar at 6–7 Mbit/s with 0.3 Mbit/s of video) and cause more loss.
//!
//! Simulation: host and viewer interceptor chains, 30 packets/s of 1200 bytes, 500 ms each
//! way, one original packet in ten lost and one retransmission in ten lost.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use rtc::interceptor::{
    AttributedPacket, Interceptor, NackGeneratorBuilder, NackResponderBuilder, Packet,
    RTCPFeedback, Registry, Slot, StreamInfo, TaggedPacket,
};
use rtc::sansio::Protocol;
use rtc::shared::TransportContext;

const SSRC: u32 = 0x1234_5678;

fn info() -> StreamInfo {
    StreamInfo {
        ssrc: SSRC,
        clock_rate: 90_000,
        mime_type: "video/H264".into(),
        payload_type: 102,
        rtcp_feedback: vec![RTCPFeedback {
            typ: "nack".into(),
            parameter: String::new(),
        }],
        ..Default::default()
    }
}

fn rtp(now: Instant, seq: u16) -> TaggedPacket {
    TaggedPacket {
        now,
        transport: TransportContext::default(),
        message: AttributedPacket::new(Packet::Rtp(rtc::rtp::Packet {
            header: rtc::rtp::header::Header {
                ssrc: SSRC,
                sequence_number: seq,
                timestamp: u32::from(seq) * 3000,
                payload_type: 102,
                ..Default::default()
            },
            payload: vec![0u8; 1200].into(),
        })),
    }
}

struct Outcome {
    originals: u32,
    resent: u32,
    duplicates: u32,
}

fn simulate(generator: NackGeneratorBuilder) -> Outcome {
    let mut host = Registry::new()
        .with(Slot::NackResponder, NackResponderBuilder::new().build())
        .build();
    let mut viewer = Registry::new()
        .with(Slot::NackGenerator, generator.build())
        .build();
    host.bind_local_stream(&info());
    viewer.bind_remote_stream(&info());

    let one_way = Duration::from_millis(500);
    let t0 = Instant::now();
    let mut to_viewer: VecDeque<(Instant, TaggedPacket)> = VecDeque::new();
    let mut to_host: VecDeque<(Instant, TaggedPacket)> = VecDeque::new();
    let mut sent: HashSet<u16> = HashSet::new();
    let mut seen: HashMap<u16, u32> = HashMap::new();
    let (mut originals, mut resent, mut retransmissions_seen) = (0u32, 0u32, 0u32);
    let mut next_seq = 0u16;
    let mut next_send = t0;
    let mut now = t0;
    while now < t0 + Duration::from_secs(10) {
        // Host: one packet every 33 ms (first transmission of one in ten is lost).
        if now >= next_send && now < t0 + Duration::from_secs(8) {
            host.handle_write(rtp(now, next_seq)).unwrap();
            next_send += Duration::from_millis(33);
            next_seq = next_seq.wrapping_add(1);
        }
        while let Some(out) = host.poll_write() {
            if let Packet::Rtp(packet) = &out.message.packet {
                let seq = packet.header.sequence_number;
                if sent.insert(seq) {
                    originals += 1;
                    if seq % 10 == 5 {
                        continue;
                    }
                } else {
                    resent += 1;
                    retransmissions_seen += 1;
                    if retransmissions_seen % 10 == 0 {
                        continue;
                    }
                }
                to_viewer.push_back((now + one_way, out));
            }
        }
        // Viewer: packets arriving, NACK timer, NACKs back to the host.
        while to_viewer.front().is_some_and(|(at, _)| *at <= now) {
            let (_, mut packet) = to_viewer.pop_front().unwrap();
            packet.now = now;
            if let Packet::Rtp(p) = &packet.message.packet {
                *seen.entry(p.header.sequence_number).or_default() += 1;
            }
            viewer.handle_read(packet).unwrap();
            while viewer.poll_read().is_some() {}
        }
        viewer.handle_timeout(now).unwrap();
        while let Some(out) = viewer.poll_write() {
            to_host.push_back((now + one_way, out));
        }
        while to_host.front().is_some_and(|(at, _)| *at <= now) {
            let (_, mut packet) = to_host.pop_front().unwrap();
            packet.now = now;
            host.handle_read(packet).unwrap();
            while host.poll_read().is_some() {}
        }
        host.handle_timeout(now).unwrap();
        now += Duration::from_millis(5);
    }
    let duplicates = seen.values().map(|n| n.saturating_sub(1)).sum();
    Outcome {
        originals,
        resent,
        duplicates,
    }
}

#[test]
fn unlimited_nacks_resend_each_loss_many_times_on_a_slow_link() {
    let default = simulate(NackGeneratorBuilder::new());
    let tuned = simulate(transport::nack_generator());
    for (name, o) in [("default", &default), ("kronos", &tuned)] {
        println!(
            "{name}: {} originals ({} lost), {} resent ({:.1} per loss), {} duplicates at the viewer",
            o.originals,
            o.originals / 10,
            o.resent,
            f64::from(o.resent) / f64::from(o.originals / 10),
            o.duplicates
        );
    }
    let lost = default.originals / 10;
    assert!(
        default.resent > lost * 5,
        "default resent {}",
        default.resent
    );
    assert!(tuned.resent <= lost * 2, "tuned resent {}", tuned.resent);
}
