pub mod admin;
pub mod error;
pub mod fields;
pub mod identity;
pub mod metadata;
pub mod session;
pub mod snapshot;
pub mod strip;

mod tests;
mod tests_boundaries;

use betterlite_config::sections::request_metadata_safety::MissingSessionIdPolicy;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::api_boundaries::ApiLimits;
use crate::api::request_destination::RequestData;
use crate::api::request_metadata_safety::admin::{
    ALLOW_CLIENT_MESSAGE_REDACTION_OPT_OUT_KEY, ALLOW_CLIENT_PRICING_OVERRIDE_KEY,
    key_or_team_metadata_flag_is_true,
};
use crate::api::request_metadata_safety::error::RequestMetadataSafetyError;
use crate::api::request_metadata_safety::metadata::{
    MetadataField, is_llm_inference_route, metadata_variable_name_for_route,
    normalized_metadata_object, normalized_metadata_slot,
};
use crate::api::request_metadata_safety::session::apply_missing_session_id_policy;
use crate::api::request_metadata_safety::strip::{
    strip_client_callback_credentials, strip_client_message_redaction_opt_out,
    strip_client_pricing_overrides, strip_router_reserved_metadata,
    strip_untrusted_request_header_controls,
};

const METADATA_SLOTS: [MetadataField; 2] =
    [MetadataField::Metadata, MetadataField::LitellmMetadata];
const HEADERS_KEY: &str = "headers";

// the fields of UserAPIKeyAuth this slice reads; only the proxy sets via_virtual_key and the admin metadata maps
#[derive(Debug, Clone, Default)]
pub struct MetadataPrincipal {
    pub api_key: Option<SecretString>,
    pub via_virtual_key: bool,
    pub metadata: Option<IndexMap<String, Json>>,
    pub team_metadata: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Clone)]
pub struct RequestMetadataSafetyInput {
    pub data: RequestData,
    pub headers: IndexMap<String, String>,
    pub route: String,
    pub principal: MetadataPrincipal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestMetadataSafetyResult {
    pub data: RequestData,
    pub headers: IndexMap<String, String>,
    pub metadata_variable_name: MetadataField,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataSafetySettings {
    pub limits: ApiLimits,
    pub missing_session_id: Option<MissingSessionIdPolicy>,
    pub turn_off_message_logging: bool,
}

impl Default for MetadataSafetySettings {
    fn default() -> Self {
        Self {
            limits: ApiLimits::default(),
            missing_session_id: None,
            turn_off_message_logging: false,
        }
    }
}

pub fn sanitize_request_metadata(
    input: &RequestMetadataSafetyInput,
    settings: &MetadataSafetySettings,
) -> Result<RequestMetadataSafetyResult, RequestMetadataSafetyError> {
    let mut data = input.data.clone();
    for field in METADATA_SLOTS {
        normalize_metadata_object_slot(&mut data, field, settings.limits)?;
    }
    let metadata_variable_name = metadata_variable_name_for_route(&input.route);
    normalized_metadata_slot(&mut data, metadata_variable_name, settings.limits);
    let mut headers = input.headers.clone();
    let allow_opt_out = key_or_team_metadata_flag_is_true(
        &input.principal,
        ALLOW_CLIENT_MESSAGE_REDACTION_OPT_OUT_KEY,
    );
    strip_untrusted_request_header_controls(&mut headers, allow_opt_out);
    apply_missing_session_id_policy(
        &mut data,
        metadata_variable_name,
        settings.missing_session_id.as_ref(),
        is_llm_inference_route(&input.route),
    );
    strip_metadata_bucket_header_controls(&mut data, allow_opt_out);
    if !key_or_team_metadata_flag_is_true(&input.principal, ALLOW_CLIENT_PRICING_OVERRIDE_KEY) {
        strip_client_pricing_overrides(&mut data);
    }
    strip_router_reserved_metadata(&mut data);
    strip_client_callback_credentials(&mut data);
    if !allow_opt_out && settings.turn_off_message_logging {
        strip_client_message_redaction_opt_out(&mut data);
    }
    Ok(RequestMetadataSafetyResult {
        data,
        headers,
        metadata_variable_name,
    })
}

// the copy of the request headers a guardrail reads lives in the metadata bucket, so it is stripped too
fn strip_metadata_bucket_header_controls(data: &mut RequestData, allow_opt_out: bool) {
    for slot in METADATA_SLOTS {
        if let Some(Json::Object(bucket)) = data.get_mut(slot.as_str())
            && let Some(Json::Object(header_values)) = bucket.get_mut(HEADERS_KEY)
        {
            strip_untrusted_request_header_controls(header_values, allow_opt_out);
        }
    }
}

fn normalize_metadata_object_slot(
    data: &mut RequestData,
    field: MetadataField,
    limits: ApiLimits,
) -> Result<(), RequestMetadataSafetyError> {
    let Some(raw) = data.get(field.as_str()) else {
        return Ok(());
    };
    if matches!(raw, Json::Null) {
        return Ok(());
    }
    let normalized = normalized_metadata_object(field, raw, limits)?;
    data.insert(field.as_str().to_owned(), Json::Object(normalized));
    Ok(())
}
