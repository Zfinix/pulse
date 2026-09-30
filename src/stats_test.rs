use std::time::Duration;

use super::*;

#[test]
fn median_empty_is_none() {
    assert_eq!(median(&[]), None);
}

#[test]
fn median_odd_count() {
    assert_eq!(median(&[9.0, 1.0, 5.0]), Some(5.0));
}

#[test]
fn median_even_count() {
    assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
}

#[test]
fn jitter_needs_two_samples() {
    assert_eq!(jitter(&[12.0]), None);
}

#[test]
fn jitter_steady_is_zero() {
    assert_eq!(jitter(&[10.0, 10.0, 10.0]), Some(0.0));
}

#[test]
fn jitter_averages_consecutive_gaps() {
    assert_eq!(jitter(&[10.0, 14.0, 11.0, 11.0]), Some(7.0 / 3.0));
}

#[test]
fn mbps_from_bytes() {
    assert_eq!(mbps(12_500_000, Duration::from_secs(1)), 100.0);
}

#[test]
fn mbps_zero_time_is_zero() {
    assert_eq!(mbps(1_000, Duration::ZERO), 0.0);
}

#[test]
fn number_scales_precision() {
    let shown: Vec<String> = [0.523, 8.44, 312.4].into_iter().map(number).collect();
    assert_eq!(shown, ["0.52", "8.4", "312"]);
}

fn full() -> Summary {
    Summary {
        download_mbps: Some(312.4),
        upload_mbps: Some(48.2),
        ping_ms: Some(12.0),
        jitter_ms: Some(3.4),
        colo: Some("LHR".into()),
    }
}

#[test]
fn summary_line_full() {
    assert_eq!(
        full().line(),
        "↓ 312 Mbps  ↑ 48 Mbps  ping 12 ms  jitter 3.4 ms  ·  LHR"
    );
}

#[test]
fn summary_line_skips_missing() {
    let summary = Summary {
        upload_mbps: None,
        colo: None,
        ..full()
    };
    assert_eq!(summary.line(), "↓ 312 Mbps  ping 12 ms  jitter 3.4 ms");
}
