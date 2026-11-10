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

// get_model_info falls back to a case-insensitive key and to the part after the provider prefix
pub(crate) fn resolved<'catalog>(
    catalog: &'catalog ModelCatalog,
    model: &str,
) -> Option<&'catalog CatalogEntry> {
    exact(catalog, model)
        .or_else(|| case_insensitive(catalog, model))
        .or_else(|| {
            let (_provider, rest) = model.split_once('/')?;
            exact(catalog, rest).or_else(|| case_insensitive(catalog, rest))
        })
}

fn case_insensitive<'catalog>(
    catalog: &'catalog ModelCatalog,
    model: &str,
) -> Option<&'catalog CatalogEntry> {
    let wanted = model.to_lowercase();
    let key = catalog
        .lookup
        .keys()
        .find(|key| key.to_lowercase() == wanted)?;
    exact(catalog, key)
}
