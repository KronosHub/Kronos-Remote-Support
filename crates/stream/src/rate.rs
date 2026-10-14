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

/// Resolution steps, in percent of the captured size. Below about 800 kbit/s even 15 fps
/// of a full-size screen are too blurry to read; fewer pixels keep text legible.
const SCALE_STEPS: [(u32, u32); 2] = [(450_000, 50), (800_000, 75)];
/// Before going back up, the bitrate must exceed the step's threshold by this much (in
/// percent): each resolution change costs a keyframe, so it must not flap.
const SCALE_UP_MARGIN: u32 = 125;
/// Minimum time between two resolution changes, down and up.
pub const SCALE_DOWN_GAP: Duration = Duration::from_secs(2);
pub const SCALE_UP_GAP: Duration = Duration::from_secs(10);

fn scale_below(bitrate: u32) -> u32 {
    SCALE_STEPS
        .iter()
        .find(|&&(threshold, _)| bitrate < threshold)
        .map_or(100, |&(_, scale)| scale)
}

/// Resolution (percent of the captured size) for a bitrate, given the current one: steps
/// down as soon as the bitrate falls under a threshold, back up only with some margin.
pub fn scale_for(bitrate: u32, current: u32) -> u32 {
    let target = scale_below(bitrate);
    if target < current {
        return target;
    }
    let reduced = u64::from(bitrate) * 100 / u64::from(SCALE_UP_MARGIN);
    scale_below(reduced as u32).max(current)
}

/// Output size for a scale, rounded down to even sizes (YUV 4:2:0).
pub fn scaled_size((width, height): (u32, u32), scale: u32) -> (u32, u32) {
    let even = |v: u32| ((v * scale / 100) & !1).max(2);
    (even(width), even(height))
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

    #[test]
    fn resolution_steps_down_at_once_and_up_with_margin() {
        assert_eq!(scale_for(4_000_000, 100), 100);
        assert_eq!(scale_for(700_000, 100), 75);
        assert_eq!(scale_for(300_000, 100), 50);
        assert_eq!(scale_for(300_000, 75), 50);
        // Just above a threshold is not enough to go back up...
        assert_eq!(scale_for(500_000, 50), 50);
        assert_eq!(scale_for(900_000, 75), 75);
        // ...25 % above is.
        assert_eq!(scale_for(570_000, 50), 75);
        assert_eq!(scale_for(1_000_000, 75), 100);
        assert_eq!(scale_for(1_000_000, 50), 100);
    }

    #[test]
    fn scaled_sizes_are_even() {
        assert_eq!(scaled_size((1920, 1080), 100), (1920, 1080));
        assert_eq!(scaled_size((1920, 1080), 75), (1440, 810));
        assert_eq!(scaled_size((1920, 1080), 50), (960, 540));
        assert_eq!(scaled_size((1366, 768), 75), (1024, 576));
        assert_eq!(scaled_size((1278, 718), 50), (638, 358));
    }
}
