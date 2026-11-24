use crate::model_catalog_lookup::error::LookupError;
use crate::model_group_lookups::error::AccessGroupError;

// Every way a catalog query stops: the deployment list B844 reads, or the access-group map B10047 builds over it.
#[derive(Debug, thiserror::Error)]
pub enum CatalogQueryError {
    #[error(transparent)]
    Lookup(#[from] LookupError),
    #[error(transparent)]
    AccessGroups(#[from] AccessGroupError),
}
