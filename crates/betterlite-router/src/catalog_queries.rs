pub mod error;
pub mod group_info;
pub mod messages;
pub mod queries;

mod tests;

use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::model_group_lookups::access_groups::AccessGroups;
use crate::model_group_lookups::group_info::GroupInfoCache;

// The read-only catalog handles one request's queries borrow: B844's lookup beside B10047's two caches.
pub struct CatalogQueries<'catalog> {
    pub lookup: CatalogLookup<'catalog>,
    pub access_groups: &'catalog AccessGroups,
    pub group_info: &'catalog GroupInfoCache,
}
