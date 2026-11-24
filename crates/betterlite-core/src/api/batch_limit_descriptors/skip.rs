use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::batch_limit_descriptors::Settings;

use super::BatchLimitDescriptors;
use super::BatchLimitError;
use super::descriptors::{
    BatchLimitDescriptor, create_batch_descriptors, has_applicable_limits,
    key_requires_access_check, project_has_io_limits,
};
use super::fields::BatchPrincipalFields;
use super::limiter::BatchLimiterSource;
use super::routing::{batch_routing_model, resolve_batch_provider};
use crate::api::model_credentials::CredentialSource;
use crate::api::request_destination::RequestData;

pub(super) struct BatchSkip {
    pub skip: bool,
    pub descriptors: Option<Vec<BatchLimitDescriptor>>,
}

const MODEL_SKIP_WARNING: &str = "general_settings.skip_batch_input_file_rate_limiting_for_models is not supported and has no effect. Use skip_batch_input_file_rate_limiting_for_providers or disable_batch_input_file_rate_limiting instead.";

// LiteLLM's fast path: the operator switch, the provider skip, then whether anything is enforceable
pub(super) fn should_skip<Limiter, Credentials>(
    service: &BatchLimitDescriptors<Limiter, Credentials>,
    fields: &BatchPrincipalFields,
    auth: &UserAPIKeyAuth,
    data: &RequestData,
    has_enqueued_scopes: bool,
) -> Result<BatchSkip, BatchLimitError<Limiter::Error, Credentials::Error>>
where
    Limiter: BatchLimiterSource,
    Credentials: CredentialSource,
{
    warn_unsupported_model_skip(&service.settings, &service.warned_unsupported_model_skip);
    if key_requires_access_check(fields) {
        return Ok(BatchSkip {
            skip: false,
            descriptors: None,
        });
    }
    if service.settings.disable_rate_limiting == Some(true) {
        return Ok(BatchSkip {
            skip: true,
            descriptors: None,
        });
    }
    let providers = service.settings.skip_providers.as_deref().unwrap_or(&[]);
    if !providers.is_empty() {
        let provider = resolve_batch_provider(service, batch_routing_model(data).as_deref())?;
        if let Some(skipped) = provider.filter(|name| providers.contains(name)) {
            tracing::debug!("Skipping batch input file processing for provider={skipped}");
            return Ok(BatchSkip {
                skip: true,
                descriptors: None,
            });
        }
    }
    let descriptors =
        create_batch_descriptors(service, fields, auth, data).map_err(BatchLimitError::Limits)?;
    if !has_enqueued_scopes && !has_applicable_limits(&descriptors) && !project_has_io_limits(auth)
    {
        tracing::debug!("Skipping batch input file processing: no rate limits configured");
        return Ok(BatchSkip {
            skip: true,
            descriptors: None,
        });
    }
    Ok(BatchSkip {
        skip: false,
        descriptors: Some(descriptors),
    })
}

// LiteLLM warns once per hook instance, the first time the configured no-op key is seen
fn warn_unsupported_model_skip(settings: &Settings, warned: &AtomicBool) {
    let configured = settings
        .skip_models
        .as_deref()
        .is_some_and(|models| !models.is_empty());
    if !configured || warned.swap(true, Ordering::Relaxed) {
        return;
    }
    tracing::warn!("{MODEL_SKIP_WARNING}");
}
