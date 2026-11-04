use std::ops::Mul as _;
use std::time::Instant;

use betterlite_config::sections::cisco_verdict_contracts::InspectionType;

use crate::cisco_configuration::payload::{
    ChatPayloadInputs, RequestMetadata, build_chat_payload, normalize_rule,
};
use crate::cisco_configuration::records::{Direction, ScanContext};
use crate::cisco_conversation::json::Json;
use crate::cisco_conversation::{Fields, InspectMessage};
use crate::cisco_inspection::construct::CiscoInspectionGuardrail;
use crate::cisco_inspection::transport::post_inspection;
use crate::cisco_verdict_enforcement::finalize::{
    FinalizeContext, FinalizeOutcome, ResponseSurface, finalize_inspection,
};
use crate::cisco_verdict_enforcement::handle_api_error;

const PROVIDER_NAME: &str = "cisco_ai_defense";

pub struct ChatInspectionArgs<'args> {
    pub messages: Vec<InspectMessage>,
    pub direction: Direction,
    pub key_user_id: Option<&'args str>,
}

// Mirrors LiteLLM's fn _inspect_chat.
pub async fn inspect_chat(
    guardrail: &CiscoInspectionGuardrail,
    args: ChatInspectionArgs<'_>,
    request_data: &mut Fields,
    response: ResponseSurface<'_>,
) -> FinalizeOutcome {
    let url = format!(
        "{}{}",
        guardrail.settings.api_base, guardrail.settings.inspect_path
    );
    let scan = ScanContext {
        surface: InspectionType::Chat,
        direction: args.direction,
    };
    let payload = build_chat_payload(chat_payload_inputs(guardrail, args, request_data));
    let start = Instant::now();
    match post_inspection(guardrail, &url, &payload, "chat").await {
        Ok(inspect_response) => {
            let duration_ms = start.elapsed().as_secs_f64().mul(1000.0_f64);
            let finalize = FinalizeContext {
                scan,
                duration_ms,
                provider_name: PROVIDER_NAME,
                guardrail_name: &guardrail.settings.guardrail_name,
                on_flagged_action: guardrail.settings.on_flagged_action,
                fallback_on_error: guardrail.settings.fallback_on_error,
            };
            finalize_inspection(&inspect_response, request_data, response, &finalize)
        }
        Err(error) => {
            tracing::debug!(error = %error, "Cisco AI Defense chat inspection failed");
            FinalizeOutcome::ApiError(handle_api_error(
                scan.surface,
                "CiscoAIDefenseGuardrailAPIError",
                guardrail.settings.fallback_on_error,
            ))
        }
    }
}

fn chat_payload_inputs(
    guardrail: &CiscoInspectionGuardrail,
    args: ChatInspectionArgs<'_>,
    request_data: &Fields,
) -> ChatPayloadInputs {
    ChatPayloadInputs {
        messages: args.messages,
        request_user: text_field(request_data, "user"),
        key_user_id: args.key_user_id.map(ToOwned::to_owned),
        litellm_call_id: text_field(request_data, "litellm_call_id"),
        request_metadata: request_metadata(request_data),
        enabled_rules: guardrail
            .settings
            .enabled_rules
            .iter()
            .map(normalize_rule)
            .collect(),
        integration_profile_id: guardrail.settings.integration_profile_id.clone(),
        integration_profile_version: guardrail.settings.integration_profile_version.clone(),
        integration_tenant_id: guardrail.settings.integration_tenant_id.clone(),
        integration_type: guardrail.settings.integration_type.clone(),
    }
}

// Mirrors LiteLLM's fn _build_metadata.
fn request_metadata(request_data: &Fields) -> RequestMetadata {
    let Json::Object(metadata) =
        Json::parse(request_data.get("metadata").map_or("null", |raw| raw.get()))
    else {
        return RequestMetadata::default();
    };
    RequestMetadata {
        src_app: text_field(&metadata, "src_app"),
        dst_app: text_field(&metadata, "dst_app"),
        src_ip: text_field(&metadata, "src_ip"),
        dst_ip: text_field(&metadata, "dst_ip"),
        dst_host: text_field(&metadata, "dst_host"),
        sni: text_field(&metadata, "sni"),
        user_agent: text_field(&metadata, "user_agent"),
    }
}

fn text_field(fields: &Fields, key: &str) -> Option<String> {
    match Json::parse(fields.get(key).map_or("null", |raw| raw.get())) {
        Json::Text(value) if !value.is_empty() => Some(value),
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => None,
    }
}

mod tests;
