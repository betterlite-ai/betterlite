use std::collections::BTreeSet;

use crate::models::settings::Check;

mod tests;

pub fn check_rules(context: &str) -> impl FnOnce(&Vec<Check>, &()) -> garde::Result {
    move |checks, ()| {
        let ids: BTreeSet<&str> = checks.iter().map(|check| check.id.as_str()).collect();
        if ids.len() != checks.len() {
            return Err(garde::Error::new("Each check must have a unique ID"));
        }
        if context.trim().is_empty() && !checks.iter().any(|check| check.enabled) {
            return Err(garde::Error::new(
                "Describe expected behavior or add an enabled check",
            ));
        }
        if ids.contains("expected_behavior") {
            return Err(garde::Error::new(
                "expected_behavior is reserved for the behavior description",
            ));
        }
        Ok(())
    }
}
