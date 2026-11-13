use betterlite_models::catalog::{CatalogEntry, ModelCatalog};

// LiteLLM's trim_messages tests the name as given against the plain model_cost dict, with no provider stripping
pub(crate) fn exact<'catalog>(
    catalog: &'catalog ModelCatalog,
    model: &str,
) -> Option<&'catalog CatalogEntry> {
    let canonical = catalog.lookup.get(model)?;
    let entry = catalog.entries.get(canonical.as_ref())?;
    Some(entry)
}

// get_model_info tries the name, its case-folded key, and then the part after the provider prefix
pub(crate) fn resolved<'catalog>(
    catalog: &'catalog ModelCatalog,
    model: &str,
) -> Option<&'catalog CatalogEntry> {
    let unprefixed = model.split_once('/').map(|(_provider, rest)| rest);
    for name in [Some(model), unprefixed].into_iter().flatten() {
        let wanted = name.to_lowercase();
        let canonical = catalog.lookup.get(name).or_else(|| {
            let folded = catalog
                .lookup
                .iter()
                .find(|(key, _)| key.to_lowercase() == wanted);
            folded.map(|(_key, canonical)| canonical)
        });
        if let Some(entry) = canonical.and_then(|name_key| catalog.entries.get(name_key.as_ref())) {
            return Some(entry);
        }
    }
    None
}
