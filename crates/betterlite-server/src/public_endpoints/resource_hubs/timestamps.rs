use chrono::NaiveDateTime;

// prisma-client-py decodes the engine's UTC timestamps into aware datetimes, and str() renders them with +00:00.
#[must_use]
pub fn utc_timestamp_text(value: NaiveDateTime) -> String {
    let base = value.format("%Y-%m-%d %H:%M:%S").to_string();
    let micros = value.and_utc().timestamp_subsec_micros();
    if micros == 0 {
        return format!("{base}+00:00");
    }
    format!("{base}.{micros:06}+00:00")
}
