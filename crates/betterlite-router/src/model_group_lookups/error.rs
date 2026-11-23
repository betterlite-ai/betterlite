use crate::model_catalog_lookup::error::LookupError;

// Every way the access-group map stops: the deployment list, or a model_info value the declared List[str] cannot hold.
#[derive(Debug, thiserror::Error)]
pub enum AccessGroupError {
    #[error(transparent)]
    Lookup(#[from] LookupError),
    #[error(
        "the deployment {model_name:?} has an access_groups value that is not a list of strings"
    )]
    InvalidAccessGroups { model_name: String },
}
