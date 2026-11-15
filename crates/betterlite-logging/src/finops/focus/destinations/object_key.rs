use crate::focus_records::destinations::FocusTimeWindow;

pub const HOURLY_FREQUENCY: &str = "hourly";

#[must_use]
pub fn build(prefix: &str, time_window: &FocusTimeWindow, filename: &str) -> String {
    let start = &time_window.start_time;
    let date = start.format("date=%Y-%m-%d").to_string();
    let mut parts = vec![prefix.to_owned(), date];
    if time_window.frequency == HOURLY_FREQUENCY {
        parts.push(start.format("hour=%H").to_string());
    }
    let key_prefix = parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<String>>()
        .join("/");
    format!("{key_prefix}/{filename}")
}
