//! Small helpers for data channels and latency statistics.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use webrtc::data_channel::{DataChannel, DataChannelEvent};

/// Waits until the channel is open.
pub async fn wait_open(channel: &Arc<dyn DataChannel>) -> Result<()> {
    loop {
        match channel.poll().await {
            Some(DataChannelEvent::OnOpen) => return Ok(()),
            Some(DataChannelEvent::OnClose) | None => bail!("data channel closed before opening"),
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
pub struct RttStats {
    pub count: usize,
    pub min_ms: f64,
    pub avg_ms: f64,
    pub max_ms: f64,
}

impl RttStats {
    pub fn from_samples(samples: &[Duration]) -> Self {
        let ms: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
        if ms.is_empty() {
            return Self::default();
        }
        Self {
            count: ms.len(),
            min_ms: ms.iter().copied().fold(f64::INFINITY, f64::min),
            avg_ms: ms.iter().sum::<f64>() / ms.len() as f64,
            max_ms: ms.iter().copied().fold(0.0, f64::max),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_from_samples() {
        let s = RttStats::from_samples(&[
            Duration::from_millis(10),
            Duration::from_millis(20),
            Duration::from_millis(30),
        ]);
        assert_eq!(s.count, 3);
        assert!((s.min_ms - 10.0).abs() < 1e-9);
        assert!((s.avg_ms - 20.0).abs() < 1e-9);
        assert!((s.max_ms - 30.0).abs() < 1e-9);
    }

    #[test]
    fn empty_stats() {
        assert_eq!(RttStats::from_samples(&[]), RttStats::default());
    }
}
