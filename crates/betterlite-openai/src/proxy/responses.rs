use betterlite_models::identity::provider_id::ProviderId;

use super::ProxyOptions;
use super::error::ProxyError;

// the LlmProviders member the proxy Responses config reports
pub const PROVIDER: ProviderId = ProviderId::LitellmProxy;
// supports_native_websocket returns False, so the OpenAI transport stays the one used
pub const SUPPORTS_NATIVE_WEBSOCKET: bool = false;

pub fn responses_url(options: &ProxyOptions, api_base: Option<&str>) -> Result<String, ProxyError> {
    let Some(base) = options.resolve_api_base(api_base) else {
        return Err(ProxyError::MissingApiBase);
    };
    Ok(format!("{}/responses", base.trim_end_matches('/')))
}
