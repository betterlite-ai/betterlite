use std::sync::Arc;

use betterlite_config::tls::Verification;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::finops::pointfive::upload::url_check::UploadUrlCheck;
use crate::posthog_logging::delivery::PooledTransport;
use crate::posthog_logging::logger::{PostHogDeps, PostHogLogger};
use crate::posthog_logging::settings::resolve;

mod tests;

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let settings = resolve(inputs.env.as_ref()).map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    let allowed_hosts = &inputs
        .config
        .sections
        .user_url_policy
        .user_url_allowed_hosts;
    let url_check: Arc<dyn UploadUrlCheck> = Arc::new(UrlValidator::new(
        SystemResolver,
        allowed_hosts,
        &Verification::Enabled,
    ));
    let logger = PostHogLogger::new(
        settings,
        PostHogDeps {
            transport: Arc::new(PooledTransport {
                client: inputs.http.clone(),
            }),
            url_check,
            ids: || uuid::Uuid::new_v4().to_string(),
            tasks: inputs.tracker.clone(),
            shutdown: Some(inputs.shutdown.clone()),
        },
    );
    Ok(logger)
}
