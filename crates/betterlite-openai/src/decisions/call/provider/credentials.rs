use std::collections::BTreeMap;

use crate::decisions::transformation::provider_config::first_secret;

use crate::decisions::call::present;

pub(super) fn combined(
    explicit: Option<&str>,
    names: &[&str],
    environment: &BTreeMap<String, String>,
) -> Option<String> {
    present(explicit)
        .map(str::to_owned)
        .or_else(|| environment_secret(names, environment))
}

fn environment_secret(names: &[&str], environment: &BTreeMap<String, String>) -> Option<String> {
    first_secret(names, &|name| {
        environment.get(name).cloned().unwrap_or_default()
    })
}
