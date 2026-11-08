use betterlite_models::catalog::{CatalogEntry, ModelCatalog};
use betterlite_models::identity::provider_id::ProviderId;

#[must_use]
pub fn provider_cost_entry<'catalog>(
    model: &str,
    provider: ProviderId,
    live: &'catalog ModelCatalog,
    bundled: &'catalog ModelCatalog,
) -> Option<&'catalog CatalogEntry> {
    let provider_model = format!("{provider}/{model}");
    for catalog in [live, bundled] {
        for key in [model, provider_model.as_str()] {
            let Some(canonical) = catalog.lookup.get(key) else {
                continue;
            };
            if let Some(entry) = catalog.entries.get(canonical.as_ref()) {
                return Some(entry);
            }
        }
    }
    None
}
