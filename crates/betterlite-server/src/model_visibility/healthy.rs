use std::collections::BTreeSet;

use async_trait::async_trait;
use betterlite_config::sections::model_visibility::Settings;

// async_get_fully_unhealthy_model_names as the listing reads it: the names every one of whose deployments a health probe marked unhealthy.
#[async_trait]
pub trait RouterHealth: Sync {
    type Error;

    async fn fully_unhealthy_model_names(&self) -> Result<BTreeSet<String>, Self::Error>;
}

// get_hidden_unhealthy_model_names: empty when the filter is off or no router exists, the router's own set otherwise, with the source's debug note on an empty one.
pub async fn hidden_unhealthy_model_names<R>(
    healthy_only: Option<bool>,
    settings: Settings,
    llm_router: Option<&R>,
) -> Result<BTreeSet<String>, R::Error>
where
    R: RouterHealth,
{
    let Some(router) = llm_router else {
        return Ok(BTreeSet::new());
    };
    // is_healthy_only_enabled: the request's healthy_only or the proxy-wide setting turns the filter on, and a request cannot turn the setting off
    let enabled = matches!(healthy_only, Some(true)) || settings.model_list_healthy_only;
    if !enabled {
        return Ok(BTreeSet::new());
    }
    let unhealthy_names = router.fully_unhealthy_model_names().await?;
    if unhealthy_names.is_empty() {
        tracing::debug!(
            "healthy-only model listing is enabled but no unhealthy deployment state is available (requires background_health_checks); returning unfiltered model list"
        );
    }
    Ok(unhealthy_names)
}
