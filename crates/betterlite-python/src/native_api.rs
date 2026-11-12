use std::collections::BTreeMap;

use betterlite_models::catalog::BundledCatalog;
use betterlite_models::provider_model_sets::ProviderModelSets;

mod error;
mod tests;

pub(crate) use error::NativeApiError;

pub(crate) const fn core_version() -> &'static str {
    // Cargo fills this from the workspace version at build time.
    env!("CARGO_PKG_VERSION")
}

pub(crate) fn bundled_model_cost_json() -> Result<String, NativeApiError> {
    let catalog = BundledCatalog::default();
    let bytes = (catalog.asset)()?;
    Ok(String::from_utf8(bytes)?)
}

pub(crate) fn provider_model_sets() -> BTreeMap<String, Vec<String>> {
    let mut sets = ProviderModelSets::default();
    sets.add_known_models(&BundledCatalog::default().get());
    sets.sets
        .into_iter()
        .map(|(set, names)| (set.to_string(), names.into_iter().collect()))
        .collect()
}
