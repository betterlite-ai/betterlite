use betterlite_logging::focus_records::destinations::FocusTimeWindow;
use tracing::warn;

#[path = "export_window/tests/empty_delivery.rs"]
mod empty_delivery;
#[path = "export_window/tests/row_cap.rs"]
mod row_cap;

// the row cap the window's data reports: reaching it means rows past it are left out, and the cap is what raises it
pub(super) fn warn_at_row_cap(limit: i64, window: &FocusTimeWindow, rows: usize) {
    let observed = i64::try_from(rows).unwrap_or(i64::MAX);
    if observed < limit {
        return;
    }
    warn!(
        "Mavvrik FOCUS export: row cap reached ({limit} rows). Some data for window {}\u{2192}{} may be excluded. Increase MAVVRIK_FOCUS_MAX_ROWS to export all rows.",
        window.start_time.date_naive(),
        window.end_time.date_naive()
    );
}
