use std::convert::Infallible;
use std::future::Future;

use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::model_catalog_keys::ModelCostKeys;

use crate::catalog_queries::CatalogQueries;
use crate::model_group_info::error::GroupInfoError;
use crate::model_group_info::group::ModelGroupInfo;
use crate::model_group_info::sources::{DeploymentInfo, GroupSources, ProviderResolver};

impl CatalogQueries<'_> {
    // Router.cached_model_group_info: the group info B10047 caches for the name, unchanged.
    pub fn cached_model_group_info<'call, D, R, P>(
        &'call self,
        sources: &'call GroupSources<'call, D, R, P>,
        model_group: &'call str,
        keys: &'call mut ModelCostKeys,
    ) -> impl Future<Output = Result<Option<ModelGroupInfo>, GroupInfoError<R::Error>>> + Send + 'call
    where
        D: DeploymentInfo + Sync,
        R: ProviderResolver + Sync,
        P: SupportedParameterSource<Error = Infallible>,
    {
        self.group_info
            .cached_get_model_group_info(&self.lookup, sources, model_group, keys)
    }
}
