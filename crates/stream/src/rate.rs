//! Adaptive quality: the host lowers the video bitrate (and frame rate) when the viewer
//! reports packet loss or growing latency, and raises it again once the link is clean.
//!
//! Loss and queueing delay (RTT above the best one seen recently) are the two signals a
//! congested path gives. Decrease is multiplicative and quick, increase is gradual and
//! waits for a quiet period, so the stream does not oscillate.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Lowest bitrate the controller goes to.
pub const MIN_BITRATE: u32 = 250_000;
/// RTT samples (one per report, ~1 s) used to estimate the uncongested RTT.
const BASE_RTT_WINDOW: usize = 30;
/// Below this many packets in a report the loss ratio is noise.
const MIN_PACKETS: u32 = 10;
/// Probe for more bandwidth only while video flows (a static screen sends almost nothing,
/// and proves nothing). Low on purpose: at the lowest rate a busy screen sends ~15/s.
const MIN_PACKETS_TO_PROBE: u32 = 5;
/// Back off at most this often: the effect of a change shows up about one report later.
const DECREASE_GAP: Duration = Duration::from_millis(1500);
/// Wait this long after a decrease before probing for more bandwidth.
const INCREASE_HOLD: Duration = Duration::from_secs(4);

/// One viewer report: counts since the previous one.
#[derive(Debug, Clone, Copy)]
pub struct Report {
    pub received: u32,
    pub lost: u32,
    /// Latest round-trip time; `None` while not measured yet.
    pub rtt: Option<Duration>,
}

#[derive(Debug)]
pub struct RateController {
    max: u32,
    current: u32,
    rtts: VecDeque<Duration>,
    last_decrease: Option<Instant>,
}

impl RateController {
    /// Starts at `max`: on a good link the best quality shows up at once.
    pub fn new(max: u32) -> Self {
        let max = max.max(MIN_BITRATE);
        Self {
            max,
            current: max,
            rtts: VecDeque::new(),
            last_decrease: None,
        }
    }

    pub fn bitrate(&self) -> u32 {
        self.current
    }

    /// Below the configured bitrate because of congestion.
    pub fn limited(&self) -> bool {
        self.current < self.max
    }

    /// Feeds a report; returns the new bitrate when it changed.
    pub fn on_report(&mut self, report: Report, now: Instant) -> Option<u32> {
        let total = report.received + report.lost;
        // One packet lost out of a dozen is not a trend.
        let loss = if total >= MIN_PACKETS && report.lost >= 2 {
            f64::from(report.lost) / f64::from(total)
        } else {
            0.0
        };
        let queueing = report.rtt.map_or(Duration::ZERO, |rtt| {
            if self.rtts.len() == BASE_RTT_WINDOW {
                self.rtts.pop_front();
            }
            self.rtts.push_back(rtt);
            let base = self.rtts.iter().min().copied().unwrap_or(rtt);
            // Lower of the last two samples: a lone spike (Wi-Fi, scheduling) is not a
            // queue, a queue lasts.
            let recent = self.rtts.iter().rev().take(2).min().copied().unwrap_or(rtt);
            recent.saturating_sub(base)
        });

        let factor = if loss > 0.10 || queueing > Duration::from_millis(300) {
            Some(0.6)
        } else if loss > 0.02 || queueing > Duration::from_millis(120) {
            Some(0.8)
        } else {
            None
        };
        let previous = self.current;
        match factor {
            Some(factor) => {
                if self
                    .last_decrease
                    .is_some_and(|t| now.duration_since(t) < DECREASE_GAP)
                {
                    return None;
                }
                self.last_decrease = Some(now);
                self.current = ((f64::from(self.current) * factor) as u32).max(MIN_BITRATE);
            }
            None => {
                let quiet = loss < 0.01 && queueing < Duration::from_millis(50);
                let held = self
                    .last_decrease
                    .is_some_and(|t| now.duration_since(t) < INCREASE_HOLD);
                if !quiet || held || report.received < MIN_PACKETS_TO_PROBE {
                    return None;
                }
                self.current = (self.current + self.current / 8 + 50_000).min(self.max);
            }
        }
        (self.current != previous).then_some(self.current)
    }
}

/// Frame rate that goes with a bitrate: below about 1.5 Mbit/s a busy screen at full rate
/// can only be encoded blurry, so fewer, sharper frames are sent instead.
pub fn fps_for(bitrate: u32, max_fps: u32) -> u32 {
    let fps = match bitrate {
        1_500_000.. => max_fps,
        800_000.. => 20,
        450_000.. => 15,
        _ => 10,
    };
    fps.min(max_fps).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn report(received: u32, lost: u32, rtt_ms: u64) -> Report {
        Report {
            received,
            lost,
            rtt: Some(MS * rtt_ms as u32),
        }
    }

    #[test]
    fn clean_link_stays_at_max() {
        let mut rc = RateController::new(4_000_000);
        let t0 = Instant::now();
        for s in 0..20 {
            assert_eq!(rc.on_report(report(300, 0, 40), t0 + MS * 1000 * s), None);
        }
        assert_eq!(rc.bitrate(), 4_000_000);
    }

    #[test]
    fn loss_backs_off_then_recovers() {
        let mut rc = RateController::new(4_000_000);
        let t0 = Instant::now();
        let at = |s: u32| t0 + MS * 1000 * s;
        rc.on_report(report(300, 0, 40), at(0));
        // Heavy loss: big step down, at most once per DECREASE_GAP.
        assert_eq!(rc.on_report(report(250, 50, 40), at(1)), Some(2_400_000));
        assert_eq!(rc.on_report(report(250, 50, 40), at(2)), None); // DECREASE_GAP
        assert_eq!(rc.on_report(report(250, 50, 40), at(3)), Some(1_440_000));
        // Mild loss.
        assert_eq!(rc.on_report(report(290, 10, 40), at(5)), Some(1_152_000));
        // Clean again: hold, then climb back to the cap.
        assert_eq!(rc.on_report(report(300, 0, 40), at(7)), None);
        let mut s = 9;
        while rc.bitrate() < 4_000_000 {
            assert!(rc.on_report(report(300, 0, 40), at(s)).is_some());
            s += 1;
        }
        assert!(s < 30, "recovery took {s} s");
    }

    #[test]
    fn growing_rtt_is_congestion() {
        let mut rc = RateController::new(4_000_000);
        let t0 = Instant::now();
        rc.on_report(report(300, 0, 40), t0);
        // A single spike is ignored, a lasting rise is a queue building up.
        assert_eq!(rc.on_report(report(300, 0, 400), t0 + MS * 1000), None);
        assert_eq!(rc.on_report(report(300, 0, 45), t0 + MS * 2000), None);
        assert_eq!(rc.on_report(report(300, 0, 200), t0 + MS * 3000), None);
        assert_eq!(
            rc.on_report(report(300, 0, 210), t0 + MS * 4000),
            Some(3_200_000)
        );
    }

    #[test]
    fn never_below_minimum_and_idle_does_not_probe() {
        let mut rc = RateController::new(4_000_000);
        let t0 = Instant::now();
        for s in 0..30 {
            rc.on_report(report(100, 100, 40), t0 + MS * 2000 * s);
        }
        assert_eq!(rc.bitrate(), MIN_BITRATE);
        // Static screen: (almost) no packets, no probing.
        let later = t0 + MS * 100_000;
        assert_eq!(rc.on_report(report(3, 0, 40), later), None);
        // Low rate but video flowing: probing resumes.
        assert!(rc.on_report(report(15, 0, 40), later).is_some());
    }

    #[test]
    fn frame_rate_follows_bitrate() {
        assert_eq!(fps_for(4_000_000, 30), 30);
        assert_eq!(fps_for(1_000_000, 30), 20);
        assert_eq!(fps_for(500_000, 30), 15);
        assert_eq!(fps_for(250_000, 30), 10);
        assert_eq!(fps_for(250_000, 5), 5);
    }
}
