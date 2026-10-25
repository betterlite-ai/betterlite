use std::collections::BTreeSet;

use crate::models::settings::LensSettings;
use crate::models::validate::RecordError;

mod tests;

pub fn check_rules(settings: &LensSettings) -> Result<(), RecordError> {
    let ids: BTreeSet<&str> = settings
        .checks
        .iter()
        .map(|check| check.id.as_str())
        .collect();
    if ids.len() != settings.checks.len() {
        return Err(RecordError::DuplicateCheck);
    }
    if settings.context.trim().is_empty() && !settings.checks.iter().any(|check| check.enabled) {
        return Err(RecordError::NoBehavior);
    }
    if ids.contains("expected_behavior") {
        return Err(RecordError::ReservedCheck);
    }
    Ok(())
}
