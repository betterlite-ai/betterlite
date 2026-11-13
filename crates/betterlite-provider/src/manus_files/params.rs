use crate::api::file_contracts::config::FileParams;

use super::error::ManusFilesError;

// A null parameter is unset, and any other non-text value is rejected instead of stringified.
pub(super) fn text<'params>(
    params: &'params FileParams,
    field: &'static str,
) -> Result<Option<&'params str>, ManusFilesError> {
    match params.get(field) {
        None => Ok(None),
        Some(value) if value.is_null() => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or(ManusFilesError::ParamNotText { field }),
    }
}
