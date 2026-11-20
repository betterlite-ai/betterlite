use std::fs;
use std::path::Path;

use num_traits::ToPrimitive as _;
use tokio_util::task::TaskTracker;

// _get_process_rss_mb reads resource.getrusage(RUSAGE_SELF).ru_maxrss, the peak RSS, which /proc reports as VmHWM
pub(crate) const RSS_STATUS_PATH: &str = "/proc/self/status";
const RSS_FIELD: &str = "VmHWM:";
const THREADS_FIELD: &str = "Threads:";
// one kB is 1/1024 of a MiB and one byte is 1/1048576, the two divisors _get_process_rss_mb applies
const KILOBYTES_TO_MEGABYTES: f64 = 0.000_976_562_5;
const BYTES_TO_MEGABYTES: f64 = 0.000_000_953_674_316_406_25;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RssUnit {
    Kilobytes,
    Bytes,
}

// The two readings the instrumentation lines report, taken from one status snapshot
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessMetrics {
    pub rss_mb: Option<f64>,
    pub thread_count: usize,
}

// ru_maxrss counts kB on Linux and bytes on macOS, which is the one platform difference the measurement keeps
#[must_use]
pub const fn host_rss_unit() -> RssUnit {
    if cfg!(target_os = "macos") {
        RssUnit::Bytes
    } else {
        RssUnit::Kilobytes
    }
}

#[must_use]
pub fn rss_mb(raw: u64, unit: RssUnit) -> f64 {
    let scale = match unit {
        RssUnit::Kilobytes => KILOBYTES_TO_MEGABYTES,
        RssUnit::Bytes => BYTES_TO_MEGABYTES,
    };
    raw.to_f64().unwrap_or_default().mul_add(scale, 0.0)
}

// A status field holds its number first, in kB for VmHWM and as a count for Threads, then a unit word
#[must_use]
pub(crate) fn status_number(status: &str, field: &str) -> Option<u64> {
    let value = status
        .lines()
        .find_map(|line| line.strip_prefix(field))?
        .split_whitespace()
        .next()?;
    value.parse().ok()
}

// resource.getrusage has no safe wrapper in this dependency set, so the Linux status file stands in and a missing field is None
#[must_use]
pub fn process_metrics(status: Option<&str>) -> ProcessMetrics {
    let peak_mb = status
        .and_then(|text| status_number(text, RSS_FIELD))
        .map(|peak| rss_mb(peak, host_rss_unit()));
    let thread_count = status
        .and_then(|text| status_number(text, THREADS_FIELD))
        .and_then(|count| usize::try_from(count).ok())
        .unwrap_or_default();
    ProcessMetrics {
        rss_mb: peak_mb,
        thread_count,
    }
}

// The async cycle reads the status file on the server's blocking pool, so no worker thread waits on the filesystem
#[must_use]
pub async fn measure_process(tasks: &TaskTracker, status_path: &Path) -> ProcessMetrics {
    let path = status_path.to_owned();
    let status = tasks
        .spawn_blocking(move || fs::read_to_string(path).ok())
        .await;
    process_metrics(status.ok().flatten().as_deref())
}

// _rss_mb_for_log: two decimals, or "unknown" when the measurement failed
#[must_use]
pub fn format_rss_mb(rss: Option<f64>) -> String {
    rss.map_or_else(
        || "unknown".to_owned(),
        |megabytes| format!("{megabytes:.2}"),
    )
}
