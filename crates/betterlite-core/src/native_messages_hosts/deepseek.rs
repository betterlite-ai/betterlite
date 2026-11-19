use betterlite_anthropic::credentials::ANTHROPIC_VERSION;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::ExposeSecret as _;

use crate::api::anthropic_beta_filter::native_injection::{NativeRequest, inject_native_betas};
use crate::api::messages_native_prepare::environment::{
    ANTHROPIC_VERSION_HEADER, CONTENT_TYPE_HEADER, JSON_CONTENT_TYPE,
};
use crate::api::messages_native_transform::capabilities::ThinkingCapabilities;
use crate::api::messages_native_transform::request::{TransformRequest, transform_native_request};

use super::error::HostError;
use super::keys::{BaseSources, KeySources, resolve_base, resolve_key};
use super::request::{EnvironmentRequest, HostRequest};

pub const CUSTOM_LLM_PROVIDER: &str = "deepseek";
pub const STRIPS_BILLING_METADATA: bool = true;
pub const DEFAULT_API_BASE: &str = "https://api.deepseek.com/anthropic";

const API_KEY_HEADER: &str = "x-api-key";
const AUTHORIZATION_HEADER: &str = "authorization";
const CUSTOM_TOOL_TYPE: &str = "custom";

#[must_use]
pub fn complete_url(sources: BaseSources<'_>) -> String {
    let resolved = resolve_base(sources, DEFAULT_API_BASE);
    let base = resolved.trim_end_matches('/');
    if base.ends_with("/v1/messages") && base.contains("/anthropic/") {
        return base.to_owned();
    }
    let base = base.strip_suffix("/v1/messages").unwrap_or(base);
    let base = base.strip_suffix("/v1").unwrap_or(base);
    let base = base.strip_suffix("/beta").unwrap_or(base);
    let mut complete = base.to_owned();
    if !complete.ends_with("/anthropic") && !complete.contains("/anthropic/") {
        complete.push_str("/anthropic");
    }
    format!("{complete}/v1/messages")
}

// DeepSeek is the one host that tolerates a missing credential and tests header names case-sensitively
pub fn validate_environment(request: EnvironmentRequest<'_>) -> Result<Option<String>, HostError> {
    let key = resolve_key(KeySources {
        explicit: request.explicit_key,
        environment: request.environment_key,
        global: request.global_key,
    });
    let headers = request.headers;
    if !headers.contains_key(API_KEY_HEADER) && !headers.contains_key(AUTHORIZATION_HEADER) {
        if let Some(key) = key {
            headers.insert(API_KEY_HEADER.to_owned(), key.expose_secret().to_owned());
        }
    }
    if !headers.contains_key(ANTHROPIC_VERSION_HEADER) {
        headers.insert(
            ANTHROPIC_VERSION_HEADER.to_owned(),
            ANTHROPIC_VERSION.to_owned(),
        );
    }
    if !headers.contains_key(CONTENT_TYPE_HEADER) {
        headers.insert(CONTENT_TYPE_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
    }
    let native = NativeRequest {
        provider: CUSTOM_LLM_PROVIDER,
        optional_params: request.optional_params,
        messages: request.messages,
    };
    *headers = inject_native_betas(headers.clone(), &native)?;
    Ok(request.api_base.map(str::to_owned))
}

pub async fn transform_request<Capabilities>(
    capabilities: &mut Capabilities,
    request: HostRequest<'_>,
) -> Result<Object, HostError>
where
    Capabilities: ThinkingCapabilities + Send,
{
    let mut body = transform_native_request(
        capabilities,
        TransformRequest {
            model: request.model,
            messages: request.messages,
            optional_params: request.optional_params,
            custom_llm_provider: CUSTOM_LLM_PROVIDER,
            strips_billing_metadata: STRIPS_BILLING_METADATA,
        },
    )
    .await?;
    sanitize_tools(&mut body);
    Ok(body)
}

// DeepSeek rejects Anthropic's explicit custom-tool discriminator; every other tool shape passes through
fn sanitize_tools(body: &mut Object) {
    let Some(Json::List(tools)) = body.get_mut("tools") else {
        return;
    };
    for tool in tools {
        let Json::Object(fields) = tool else {
            continue;
        };
        if fields.get("type").and_then(Json::as_str) == Some(CUSTOM_TOOL_TYPE) {
            fields.shift_remove("type");
        }
    }
}
