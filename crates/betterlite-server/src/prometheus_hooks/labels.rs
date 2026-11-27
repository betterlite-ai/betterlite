use std::collections::BTreeMap;

use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::fields::payload::records::StandardLoggingMetadata;
use betterlite_logging::prometheus::failure::exception::LabelException;
use betterlite_logging::prometheus_label_projection::combined_custom_metadata::{
    CombinedMetadataError, combined_custom_metadata,
};
use betterlite_logging::prometheus_label_projection::custom_metadata_extraction::custom_labels_from_metadata;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_models::decisions::Json;
use betterlite_models::key_digest::hashing::hash_token;
use betterlite_models::projections::service_tier::resolved_service_tier;
use betterlite_models::zero_cost::{
    AUDIO_INPUT_RATE, AUDIO_OUTPUT_RATE, TEXT_INPUT_RATE, TEXT_OUTPUT_RATE, ZeroCostDiagnostic,
    ZeroCostReason,
};
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;

use super::json::{as_object, as_text};

// LiteLLM builds this payload's labels from its metadata dict, so a typed field that is absent reads as None
pub(crate) fn success_values(
    settings: &PrometheusLabelSettings,
    payload: &StandardLoggingPayload,
    model: &str,
    end_user: Option<String>,
) -> Result<UserApiKeyLabelValues, CombinedMetadataError> {
    let metadata = &payload.metadata;
    let stream = settings
        .emit_stream_label
        .then(|| payload_stream_text(payload));
    Ok(UserApiKeyLabelValues {
        end_user,
        user: metadata.user_api_key_user_id.clone(),
        user_email: metadata.user_api_key_user_email.clone(),
        hashed_api_key: metadata.user_api_key_hash.clone(),
        api_key_alias: metadata.user_api_key_alias.clone(),
        team: metadata.user_api_key_team_id.clone(),
        team_alias: metadata.user_api_key_team_alias.clone(),
        model_group: payload.model_group.clone(),
        requested_model: payload.model_group.clone(),
        model: Some(model.to_owned()),
        litellm_model_name: Some(model.to_owned()),
        tags: text_tags(&payload.request_tags),
        custom_metadata_labels: custom_labels(Some(metadata), settings)?,
        model_id: payload.model_id.clone(),
        api_base: Some(payload.api_base.clone()),
        api_provider: payload.custom_llm_provider.clone(),
        status_code: Some("200".to_owned()),
        route: metadata.user_api_key_request_route.clone(),
        client_ip: metadata.requester_ip_address.clone(),
        user_agent: metadata.user_agent.clone(),
        stream,
        org_id: metadata.user_api_key_org_id.clone(),
        org_alias: metadata.user_api_key_org_alias.clone(),
        service_tier: service_tier(payload).map(str::to_owned),
        ..UserApiKeyLabelValues::default()
    })
}

// the failure event's smaller set: no status, no exception, no route, no service tier
pub(crate) fn failure_values(
    settings: &PrometheusLabelSettings,
    payload: Option<&StandardLoggingPayload>,
    model: &str,
    end_user: Option<String>,
) -> Result<UserApiKeyLabelValues, CombinedMetadataError> {
    let metadata = payload.map(|record| &record.metadata);
    Ok(UserApiKeyLabelValues {
        end_user,
        hashed_api_key: metadata.and_then(|fields| fields.user_api_key_hash.clone()),
        api_key_alias: metadata.and_then(|fields| fields.user_api_key_alias.clone()),
        model: Some(model.to_owned()),
        team: metadata.and_then(|fields| fields.user_api_key_team_id.clone()),
        team_alias: metadata.and_then(|fields| fields.user_api_key_team_alias.clone()),
        user: metadata.and_then(|fields| fields.user_api_key_user_id.clone()),
        model_id: payload.map(|record| record.model_id.clone().unwrap_or_default()),
        requested_model: payload.and_then(|record| record.model_group.clone()),
        api_provider: payload.and_then(|record| record.custom_llm_provider.clone()),
        custom_metadata_labels: custom_labels(metadata, settings)?,
        ..UserApiKeyLabelValues::default()
    })
}

fn custom_labels(
    metadata: Option<&StandardLoggingMetadata>,
    settings: &PrometheusLabelSettings,
) -> Result<BTreeMap<String, String>, CombinedMetadataError> {
    let combined = combined_custom_metadata(metadata)?;
    Ok(custom_labels_from_metadata(
        &combined,
        &settings.custom_metadata_labels,
    ))
}

// the typed error information stands in for the exception object the kwargs would carry
pub(crate) fn payload_exception(payload: &StandardLoggingPayload) -> Option<LabelException> {
    let info = payload.error_information.as_ref()?;
    Some(LabelException {
        class_name: info
            .error_class
            .as_ref()
            .and_then(Option::as_deref)
            .unwrap_or_default()
            .to_owned(),
        message: info
            .error_message
            .as_ref()
            .and_then(Option::as_deref)
            .map(str::to_owned),
        status_code: info
            .error_code
            .as_ref()
            .and_then(Option::as_deref)
            .map(str::to_owned),
        llm_provider: info
            .llm_provider
            .as_ref()
            .and_then(Option::as_deref)
            .map(str::to_owned),
        category: info
            .error_rate_limit_category
            .as_ref()
            .and_then(Option::as_deref)
            .map(str::to_owned),
        rate_limit_type: info
            .error_rate_limit_type
            .as_ref()
            .and_then(Option::as_deref)
            .map(str::to_owned),
        ..LabelException::default()
    })
}

pub(crate) fn zero_cost_diagnostic(payload: &StandardLoggingPayload) -> Option<ZeroCostDiagnostic> {
    let fields = payload.zero_cost_diagnostic.as_ref().and_then(as_object)?;
    let reason = match fields.get("reason").and_then(as_text)? {
        "missing_pricing_key" => ZeroCostReason::MissingPricingKey,
        "pricing_not_applied" => ZeroCostReason::PricingNotApplied,
        "cost_calculation_error" => ZeroCostReason::CostCalculationError,
        _other => return None,
    };
    let pricing_model = fields
        .get("pricing_model")
        .and_then(as_text)
        .unwrap_or_default()
        .to_owned();
    let mut missing = Vec::new();
    if let Some(Json::List(items)) = fields.get("missing_pricing_keys") {
        for item in items {
            let key = as_text(item).unwrap_or_default();
            for known in [
                TEXT_INPUT_RATE,
                AUDIO_INPUT_RATE,
                TEXT_OUTPUT_RATE,
                AUDIO_OUTPUT_RATE,
            ] {
                if known == key {
                    missing.push(known);
                }
            }
        }
    }
    Some(ZeroCostDiagnostic {
        reason,
        pricing_model,
        missing_pricing_keys: missing,
    })
}

// litellm reads response.service_tier, the usage object's service_tier, then the requested tier that survives the known list
pub(crate) fn service_tier(payload: &StandardLoggingPayload) -> Option<&str> {
    let response = payload
        .response
        .as_ref()
        .and_then(as_object)
        .and_then(|fields| fields.get("service_tier"))
        .and_then(as_text)
        .filter(|tier| !tier.is_empty());
    let usage = payload
        .metadata
        .usage_object
        .as_ref()
        .and_then(as_object)
        .and_then(|fields| fields.get("service_tier"))
        .and_then(as_text)
        .filter(|tier| !tier.is_empty());
    let requested = payload
        .model_parameters
        .get("service_tier")
        .and_then(as_text);
    resolved_service_tier(response, usage, requested)
}

pub(crate) fn hashed_key(key: Option<&str>) -> Option<String> {
    key.map(|token| {
        if token.starts_with("sk-") {
            hash_token(token)
        } else {
            token.to_owned()
        }
    })
}

pub(crate) fn text_tags(tags: &[Json]) -> Vec<String> {
    let mut labels = Vec::new();
    for tag in tags {
        if let Json::Text(text) = tag {
            labels.push(text.clone());
        }
    }
    labels
}

pub(crate) fn stream_text(stream: bool) -> String {
    if stream {
        "True".to_owned()
    } else {
        "False".to_owned()
    }
}

fn payload_stream_text(payload: &StandardLoggingPayload) -> String {
    payload
        .stream
        .map_or_else(|| "None".to_owned(), stream_text)
}
