#![cfg(test)]

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use rstest::rstest;

use crate::health_cycle::rss::{
    RSS_STATUS_PATH, RssUnit, host_rss_unit, rss_mb, rss_mb_from, status_number, thread_count,
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

#[rstest]
#[case::missing_field("Name:\tbetterlite\nThreads:\t4\n")]
#[case::malformed_value("VmHWM:\t soon kB\n")]
#[case::negative_value("VmHWM:\t -5 kB\n")]
#[case::empty("")]
fn an_unusable_status_has_no_measurement(#[case] status: &str) {
    assert_eq!(status_number(status, "VmHWM:"), None);
}

#[test]
fn an_unreadable_status_has_no_measurement() {
    assert_eq!(rss_mb_from(Path::new("")), None);
}

#[cfg(target_os = "linux")]
#[test]
fn a_live_process_reports_its_peak_from_vmhwm() {
    let measured = rss_mb_from(Path::new(RSS_STATUS_PATH)).expect("linux exposes a measurement");
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
        let counted = thread_count();
        release.store(true, Ordering::Release);
        parked.thread().unpark();
        parked.join().expect("the helper thread stops");
        assert!(counted >= 2, "the helper thread is counted: {counted}");
    });
}
