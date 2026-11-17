use std::sync::Arc;

use betterlite_config::tls::Verification;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::finops::pointfive::logger::client::PointFiveService;
use crate::finops::pointfive::logger::{PointFiveDeps, PointFiveLogger};
use crate::finops::pointfive::upload::transport::PointFiveHttp;

mod tests;

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let config = &inputs.config;
    let section = &config.sections.pointfive_params;
    let params = section.params.as_deref().cloned().unwrap_or_default();
    let policy = &config.sections.user_url_policy;
    let source = PointFiveService::new(
        Arc::new(PointFiveHttp {
            client: inputs.http.clone(),
        }),
        Arc::new(UrlValidator::new(
            SystemResolver,
            &policy.user_url_allowed_hosts,
            &Verification::Enabled,
        )),
        Arc::clone(&inputs.env),
    );
    let logger = PointFiveLogger::new(
        params,
        PointFiveDeps {
            source: Arc::new(source),
            given_upload_client: None,
            global_message_redaction: config.sections.logging_privacy.turn_off_message_logging,
            start_periodic_flush: true,
            tasks: inputs.tracker.clone(),
            shutdown: inputs.shutdown.clone(),
        },
    )
    .map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    Ok(logger)
}
