use betterlite_context::anthropic_catalog_lookup::error::CatalogLookupError as CapabilityError;

use crate::model_catalog_lookup::error::LookupError;
use crate::model_group_info::cost::CostError;

// Every way get_model_group_info stops: the deployment list, the metadata, the provider answer, the cost text.
#[derive(Debug, thiserror::Error)]
pub enum GroupInfoError {
    #[error(transparent)]
    Lookup(#[from] LookupError),
    #[error(transparent)]
    Cost(#[from] CostError),
    #[error("the model_group_alias entry {alias:?} has a model value that is not text")]
    AliasModel { alias: String },
    #[error("the deployment field {field} does not have the type ModelGroupInfo declares")]
    Field { field: &'static str },
    #[error("the deployment rate limit total overflowed i64")]
    Totals,
    #[error("resolving the deployment provider failed: {message}")]
    Provider { message: String },
    #[error("the fast-mode capability lookup failed: {source}")]
    FastMode {
        #[source]
        source: CapabilityError,
    },
    #[error("reading the deployment supported OpenAI parameters failed: {message}")]
    Parameters { message: String },
}
