use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use http::HeaderMap;
use url::Url;

use crate::cache_aware_routing::AnthropicCacheRouting;
use crate::cache_aware_routing::validation;
use crate::cache_prediction_targets::headers::supported_prediction_headers;
use crate::cache_prediction_targets::messages_url::MESSAGES_PATH;

impl AnthropicCacheRouting {
    #[must_use]
    pub fn request_body(
        url: &str,
        headers: &HeaderMap,
        body: &JsonObject,
        request_kwargs: &JsonObject,
        messages: Option<&Json>,
    ) -> Option<Json> {
        if !url_path(url).ends_with(MESSAGES_PATH) || !supported_prediction_headers(headers) {
            return None;
        }
        Some(validation::native_body(body, request_kwargs, messages))
    }
}

// urlparse().path keeps everything before the query and fragment, whether or not the URL names a scheme.
fn url_path(url: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        let end = url.find(['?', '#']).unwrap_or(url.len());
        return url.get(..end).unwrap_or_default().to_owned();
    };
    parsed.path().to_owned()
}

mod tests;
