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
    let mut candidates = vec![model];
    if let Some((_provider, rest)) = model.split_once('/') {
        candidates.push(rest);
    }
    for candidate in candidates {
        let found = exact(catalog, candidate).or_else(|| case_insensitive(catalog, candidate));
        if found.is_some() {
            return found;
        }
    }
    None
}

fn case_insensitive<'catalog>(
    catalog: &'catalog ModelCatalog,
    model: &str,
) -> Option<&'catalog CatalogEntry> {
    let wanted = model.to_lowercase();
    for (key, canonical) in &catalog.lookup {
        if key.to_lowercase() == wanted {
            return catalog.entries.get(canonical.as_ref());
        }
    }
    None
}
