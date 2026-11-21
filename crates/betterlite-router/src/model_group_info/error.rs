use std::convert::Infallible;
use std::error::Error;

use betterlite_context::anthropic_catalog_lookup::error::CatalogLookupError as CapabilityError;

use crate::model_catalog_lookup::error::LookupError;
use crate::model_group_info::cost::CostError;

// Every way get_model_group_info stops: the deployment list, the fields, the provider answer, the cost text.
#[derive(Debug, thiserror::Error)]
pub enum GroupInfoError<ProviderError = Infallible>
where
    ProviderError: Error + 'static,
{
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
    #[error("{source}")]
    Provider {
        #[source]
        source: ProviderError,
    },
    #[error("the fast-mode capability lookup failed")]
    FastMode(#[from] CapabilityError),
}

// The field and cost helpers carry no provider error, so lifting one only fills the enum's parameter.
pub(crate) fn lift<ProviderError>(failure: GroupInfoError) -> GroupInfoError<ProviderError>
where
    ProviderError: Error + 'static,
{
    match failure {
        GroupInfoError::Lookup(error) => GroupInfoError::Lookup(error),
        GroupInfoError::Cost(error) => GroupInfoError::Cost(error),
        GroupInfoError::AliasModel { alias } => GroupInfoError::AliasModel { alias },
        GroupInfoError::Field { field } => GroupInfoError::Field { field },
        GroupInfoError::Totals => GroupInfoError::Totals,
        GroupInfoError::Provider { source } => match source {},
        GroupInfoError::FastMode(error) => GroupInfoError::FastMode(error),
    }
}
