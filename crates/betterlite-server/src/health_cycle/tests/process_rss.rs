use std::fs::read_to_string;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use rstest::rstest;

use crate::health_cycle::rss::{
    RssUnit, host_rss_unit, parse_rss_mb, process_rss_mb, rss_mb, rss_mb_from, thread_count,
};

#[rstest]
#[case::linux_kilobytes(2048, RssUnit::Kilobytes, 2.0_f64)]
#[case::linux_fractional(1536, RssUnit::Kilobytes, 1.5_f64)]
#[case::macos_bytes(2_097_152, RssUnit::Bytes, 2.0_f64)]
#[case::macos_fractional(1_048_576, RssUnit::Bytes, 1.0_f64)]
fn scaling_uses_the_getrusage_unit(raw: u64, unit: RssUnit, expected: f64) {
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
    let measured = parse_rss_mb(status, RssUnit::Kilobytes).expect("VmHWM is present");
    assert_eq!(measured.to_bits(), 1.0_f64.to_bits());
}

#[test]
fn vmhwm_scales_as_bytes_when_the_platform_says_so() {
    let status = "VmHWM:\t   1048576 kB\n";
    let measured = parse_rss_mb(status, RssUnit::Bytes).expect("VmHWM is present");
    assert_eq!(measured.to_bits(), 1.0_f64.to_bits());
}

#[rstest]
#[case::missing_field("Name:\tbetterlite\nThreads:\t4\n")]
#[case::malformed_value("VmHWM:\t soon kB\n")]
#[case::negative_value("VmHWM:\t -5 kB\n")]
#[case::empty("")]
fn an_unusable_status_has_no_measurement(status: &str) {
    assert_eq!(parse_rss_mb(status, RssUnit::Kilobytes), None);
}

#[test]
fn an_unreadable_status_has_no_measurement() {
    assert_eq!(rss_mb_from(Path::new("")), None);
}

#[cfg(target_os = "linux")]
#[test]
fn a_live_process_reports_its_peak_from_vmhwm() {
    let status = read_to_string("/proc/self/status").expect("the status file is readable");
    let line = status
        .lines()
        .find(|line| line.starts_with("VmHWM:"))
        .expect("the status file carries VmHWM");
    let raw: u64 = line
        .split_whitespace()
        .nth(1)
        .expect("VmHWM carries a value")
        .parse()
        .expect("VmHWM is a number");
    let measured = process_rss_mb().expect("linux exposes a measurement");
    assert_eq!(
        measured.to_bits(),
        rss_mb(raw, RssUnit::Kilobytes).to_bits()
    );
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
