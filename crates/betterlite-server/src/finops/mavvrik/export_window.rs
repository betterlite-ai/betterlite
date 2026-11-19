use betterlite_logging::focus_records::destinations::FocusTimeWindow;
use tracing::warn;

use super::error::MavvrikError;
use super::export::{MavvrikEngineFactory, MavvrikExporter, MavvrikFocusLogger};

#[path = "export_window/tests/empty_delivery.rs"]
mod empty_delivery;
#[path = "export_window/tests/row_cap.rs"]
mod row_cap;

impl<F: MavvrikEngineFactory> MavvrikFocusLogger<F> {
    // _export_window: the row cap warns, an empty export is still delivered, and the window reaches the destination
    pub async fn export_window(
        &self,
        window: &FocusTimeWindow,
        limit: Option<i64>,
    ) -> Result<(), MavvrikError<<F::Engine as MavvrikExporter>::Error>> {
        let effective_limit = limit.unwrap_or(self.max_rows);
        let engine = self.ensure_engine().map_err(MavvrikError::Export)?;
        let rows = engine
            .usage_rows(effective_limit, window.start_time, window.end_time)
            .await
            .map_err(MavvrikError::Export)?;
        warn_at_row_cap(effective_limit, window, rows.len());
        let payload = if rows.is_empty() {
            tracing::debug!("Mavvrik FOCUS export: no usage data for window {window:?}");
            Vec::new()
        } else {
            engine.payload(&rows).map_err(MavvrikError::Export)?
        };
        let filename = engine.filename(window).map_err(MavvrikError::Export)?;
        engine
            .deliver(&payload, window, &filename)
            .await
            .map_err(MavvrikError::Export)
    }
}

// the row cap the window's data reports: reaching it means rows past it are left out, and the cap is what raises it
fn warn_at_row_cap(limit: i64, window: &FocusTimeWindow, rows: usize) {
    let observed = i64::try_from(rows).unwrap_or(i64::MAX);
    if observed < limit {
        return;
    }
    warn!(
        "Mavvrik FOCUS export: row cap reached ({limit} rows). Some data for window {}→{} may be excluded. Increase MAVVRIK_FOCUS_MAX_ROWS to export all rows.",
        window.start_time.date(),
        window.end_time.date()
    );
}
