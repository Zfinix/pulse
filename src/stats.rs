//! The arithmetic behind a result: medians, jitter, megabits per second, and
//! the summary line a run ends on.

use std::time::Duration;

/// The middle of `samples`, or the mean of the two middles for an even count.
pub fn median(samples: &[f64]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    match sorted.len() % 2 {
        0 => Some((sorted[mid - 1] + sorted[mid]) / 2.0),
        _ => Some(sorted[mid]),
    }
}

/// Mean absolute difference between consecutive samples.
pub fn jitter(samples: &[f64]) -> Option<f64> {
    if samples.len() < 2 {
        return None;
    }
    let total: f64 = samples
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .sum();
    Some(total / (samples.len() - 1) as f64)
}

/// Megabits per second for `bytes` moved in `elapsed`.
pub fn mbps(bytes: u64, elapsed: Duration) -> f64 {
    let secs = elapsed.as_secs_f64();
    if secs <= 0.0 {
        return 0.0;
    }
    bytes as f64 * 8.0 / secs / 1_000_000.0
}

/// A reading as people say it: two decimals under 1, one under 10, whole above.
pub fn number(value: f64) -> String {
    match value {
        v if v < 1.0 => format!("{v:.2}"),
        v if v < 10.0 => format!("{v:.1}"),
        v => format!("{v:.0}"),
    }
}

/// What a finished run measured.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub download_mbps: Option<f64>,
    pub ping_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
}

impl Summary {
    /// `↓ 312 Mbps  ping 12 ms  jitter 3 ms`.
    pub fn line(&self) -> String {
        let mut parts = Vec::new();
        if let Some(down) = self.download_mbps {
            parts.push(format!("↓ {} Mbps", number(down)));
        }
        if let Some(ping) = self.ping_ms {
            parts.push(format!("ping {} ms", number(ping)));
        }
        if let Some(jitter) = self.jitter_ms {
            parts.push(format!("jitter {} ms", number(jitter)));
        }
        parts.join("  ")
    }
}

#[cfg(test)]
#[path = "stats_test.rs"]
mod tests;
