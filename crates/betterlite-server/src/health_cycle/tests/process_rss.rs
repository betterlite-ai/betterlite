#![cfg(test)]

#[cfg(target_os = "linux")]
use std::fs;
use std::path::Path;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "linux")]
use std::thread;

use rstest::rstest;
use tokio_util::task::TaskTracker;

#[cfg(target_os = "linux")]
use crate::health_cycle::rss::RSS_STATUS_PATH;
use crate::health_cycle::rss::{
    RssUnit, host_rss_unit, measure_process, process_metrics, rss_mb, status_number,
};

#[rstest]
#[case::linux_kilobytes(2048, RssUnit::Kilobytes, 2.0_f64)]
#[case::linux_fractional(1536, RssUnit::Kilobytes, 1.5_f64)]
#[case::macos_bytes(0x0020_0000, RssUnit::Bytes, 2.0_f64)]
#[case::macos_fractional(0x0010_0000, RssUnit::Bytes, 1.0_f64)]
fn scaling_uses_the_getrusage_unit(#[case] raw: u64, #[case] unit: RssUnit, #[case] expected: f64) {
    assert_eq!(rss_mb(raw, unit).to_bits(), expected.to_bits());
}

#[test]
fn the_host_unit_follows_the_platform() {
    let expected = if cfg!(target_os = "macos") {
        RssUnit::Bytes
    } else {
        RssUnit::Kilobytes
    };
    assert_eq!(host_rss_unit(), expected);
}

#[test]
fn vmhwm_is_read_as_kilobytes() {
    let status = "Name:\tbetterlite\nVmHWM:\t   1024 kB\nThreads:\t4\n";
    let peak = status_number(status, "VmHWM:").expect("VmHWM is present");
    assert_eq!(peak, 1024);
    assert_eq!(
        rss_mb(peak, RssUnit::Kilobytes).to_bits(),
        1.0_f64.to_bits()
    );
}

#[test]
fn vmhwm_scales_as_bytes_when_the_platform_says_so() {
    let status = "VmHWM:\t   1048576 kB\n";
    let peak = status_number(status, "VmHWM:").expect("VmHWM is present");
    assert_eq!(rss_mb(peak, RssUnit::Bytes).to_bits(), 1.0_f64.to_bits());
}

#[test]
fn one_status_read_carries_both_metrics() {
    let status = "Name:\tbetterlite\nVmHWM:\t   2048 kB\nThreads:\t4\n";
    let metrics = process_metrics(Some(status));
    assert_eq!(metrics.rss_mb.map(f64::to_bits), Some(2.0_f64.to_bits()));
    assert_eq!(metrics.thread_count, 4);
}

#[test]
fn a_status_without_the_fields_has_no_metrics() {
    let metrics = process_metrics(Some("Name:\tbetterlite\n"));
    assert_eq!(metrics.rss_mb, None);
    assert_eq!(metrics.thread_count, 0);
}

#[rstest]
#[case::missing_field("Name:\tbetterlite\nThreads:\t4\n")]
#[case::malformed_value("VmHWM:\t soon kB\n")]
#[case::negative_value("VmHWM:\t -5 kB\n")]
#[case::empty("")]
fn an_unusable_status_has_no_measurement(#[case] status: &str) {
    assert_eq!(status_number(status, "VmHWM:"), None);
}

#[tokio::test]
async fn an_unreadable_status_has_no_measurement() {
    let metrics = measure_process(&TaskTracker::new(), Path::new("")).await;
    assert_eq!(metrics.rss_mb, None);
    assert_eq!(metrics.thread_count, 0);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_live_process_reports_its_peak_from_vmhwm() {
    let metrics = measure_process(&TaskTracker::new(), Path::new(RSS_STATUS_PATH)).await;
    let measured = metrics.rss_mb.expect("linux exposes a measurement");
    assert!(measured > 0.0_f64, "a live peak is positive: {measured}");
}

#[cfg(target_os = "linux")]
#[test]
fn a_live_thread_count_sees_the_os_threads() {
    let release = AtomicBool::new(false);
    thread::scope(|scope| {
        let parked = scope.spawn(|| {
            while !release.load(Ordering::Acquire) {
                thread::park();
            }
        });
        let status = fs::read_to_string(RSS_STATUS_PATH).expect("linux exposes a status file");
        let counted = process_metrics(Some(&status)).thread_count;
        release.store(true, Ordering::Release);
        parked.thread().unpark();
        parked.join().expect("the helper thread stops");
        assert!(counted >= 2, "the helper thread is counted: {counted}");
    });
}
