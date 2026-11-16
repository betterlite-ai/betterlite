use crate::focus_records::destinations::FocusTimeWindow;

const TIMESTAMP_FORMAT: &str = "%Y%m%dT%H%M%SZ";

#[derive(Debug, thiserror::Error)]
pub enum FilenameError {
    #[error("Serializer must declare a file extension")]
    MissingExtension,
}

pub fn build(window: &FocusTimeWindow, extension: &str) -> Result<String, FilenameError> {
    if extension.is_empty() {
        return Err(FilenameError::MissingExtension);
    }
    let start = window.start_time.format(TIMESTAMP_FORMAT);
    let end = window.end_time.format(TIMESTAMP_FORMAT);
    Ok(format!("usage_{start}_{end}.{extension}"))
}
