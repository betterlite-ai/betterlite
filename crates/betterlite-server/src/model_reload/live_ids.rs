use std::collections::BTreeSet;

use betterlite_router::catalog_updates::catalog::CatalogHandle;

// live_model_ids_snapshot: the deployment ids this pod serves right now; an absent router serves nothing.
#[must_use]
pub fn live_model_ids_snapshot(catalog: Option<&CatalogHandle>) -> BTreeSet<String> {
    catalog.map_or_else(BTreeSet::new, |handle| {
        handle.load().id_index.keys().cloned().collect()
    })
}
