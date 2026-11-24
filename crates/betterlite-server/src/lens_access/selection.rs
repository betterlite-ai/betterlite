use betterlite_lens::models::settings::LensSettings;
use betterlite_lens::sources::parse_execution::parse_execution;

use crate::lens_access::error::LensAccessError;

pub(crate) fn validate_selection(settings: &LensSettings) -> Result<(), LensAccessError> {
    for identity in &settings.execution_ids {
        match parse_execution(identity) {
            Ok((source, _, _, _)) => {
                if !matches!(source.as_str(), "traces" | "requests") {
                    return Err(LensAccessError::InvalidSelection);
                }
            }
            Err(_error) => return Err(LensAccessError::InvalidSelection),
        }
    }
    Ok(())
}

mod tests;
