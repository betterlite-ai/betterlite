use std::time::Duration;

use super::client::{ClientKind, ClientSettings, ImageVariationClient, select_client};
use super::error::{ImageVariationResult, ImageVariationSource, OpenAiImageVariationError};
use super::{ImageVariationData, ImageVariationRequest, OpenAiImageVariationConfig, PROVIDER};
use crate::api::image_contracts::records::ImageHeaders;

// the OpenAI SDK's own default read timeout, since LiteLLM passes a missing timeout through as None
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug)]
pub struct PreparedImageVariation {
    pub data: ImageVariationData,
    pub client: ImageVariationClient,
    pub headers: Option<ImageHeaders>,
    pub deadline: Duration,
    pub boundary: String,
    pub now_seconds: i64,
    pub async_mode: bool,
}

// image_variations before its first call: config, max_retries, transform, data and client, in that order
pub fn prepare_image_variation(
    request: ImageVariationRequest,
) -> ImageVariationResult<PreparedImageVariation> {
    if request.config.is_none() {
        return Err(OpenAiImageVariationError::wrapped(
            ImageVariationSource::MissingConfig { provider: PROVIDER },
        ));
    }
    let mut optional_params = request.optional_params;
    let max_retries = match super::take_max_retries(&mut optional_params) {
        Ok(value) => value,
        Err(source) => return Err(OpenAiImageVariationError::wrapped(source)),
    };
    let Some(data) = OpenAiImageVariationConfig::request_data(request.image, optional_params)
    else {
        return Err(OpenAiImageVariationError::wrapped(
            ImageVariationSource::MissingData,
        ));
    };
    let kind = if request.params.async_call {
        ClientKind::Async
    } else {
        ClientKind::Sync
    };
    let settings = ClientSettings {
        kind,
        api_key: request.api_key.as_ref(),
        api_base: request.api_base.as_deref(),
        organization: request.organization.as_deref(),
        timeout: request.timeout,
        max_retries,
        supplied: request.client.as_ref(),
        shared_session: request.shared_session.is_some(),
    };
    let client = match select_client(&settings) {
        Ok(client) => client,
        Err(source) => return Err(OpenAiImageVariationError::wrapped(source)),
    };
    let deadline = client.timeout.unwrap_or(DEFAULT_TIMEOUT);
    Ok(PreparedImageVariation {
        data,
        client,
        headers: request.headers,
        deadline,
        boundary: request.boundary,
        now_seconds: request.now_seconds,
        async_mode: request.params.async_call,
    })
}
