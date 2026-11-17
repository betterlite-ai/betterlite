use std::sync::Arc;

use betterlite_config::tls::Verification;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::finops::pointfive::logger::client::PointFiveService;
use crate::finops::pointfive::logger::{PointFiveDeps, PointFiveLogger};
use crate::finops::pointfive::upload::transport::PointFiveHttp;
use crate::finops::pointfive::upload::url_check::UploadUrlCheck;

mod tests;

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let params = inputs
        .config
        .sections
        .pointfive_params
        .params
        .clone()
        .unwrap_or_default();
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
    let source = PointFiveService::new(
        Arc::new(PointFiveHttp {
            client: inputs.http.clone(),
        }),
        url_check,
        Arc::clone(&inputs.env),
    );
    let logger = PointFiveLogger::new(
        params,
        PointFiveDeps {
            source: Arc::new(source),
            given_upload_client: None,
            global_message_redaction: inputs
                .config
                .sections
                .logging_privacy
                .turn_off_message_logging,
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
