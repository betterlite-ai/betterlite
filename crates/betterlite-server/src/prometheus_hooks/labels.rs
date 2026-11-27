use std::collections::BTreeMap;

use betterlite_error::normalize::class::Class;
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
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};

use super::json::{as_object, as_text};

// LiteLLM builds this payload's labels from its metadata dict, so a typed field that is absent reads as None
pub(crate) struct EventLabels<'labels> {
    pub(crate) settings: &'labels PrometheusLabelSettings,
    pub(crate) payload: Option<&'labels StandardLoggingPayload>,
    pub(crate) model: &'labels str,
    pub(crate) end_user: Option<String>,
    pub(crate) success: bool,
}

pub(crate) fn event_values(
    labels: &EventLabels<'_>,
) -> Result<UserApiKeyLabelValues, CombinedMetadataError> {
    let mut entries = common_entries(labels);
    if labels.success {
        success_entries(&mut entries, labels);
    } else if let Some(record) = labels.payload {
        push_text(
            &mut entries,
            "model_id",
            Some(record.model_id.as_deref().unwrap_or_default()),
        );
    }
    let metadata = labels.payload.map(|record| &record.metadata);
    let custom = custom_labels(metadata, labels.settings)?;
    entries.push((
        "custom_metadata_labels".to_owned(),
        LabelDumpValue::Metadata(custom),
    ));
    Ok(UserApiKeyLabelValues::from_entries(&entries))
}

fn common_entries(labels: &EventLabels<'_>) -> Vec<(String, LabelDumpValue)> {
    let metadata = labels.payload.map(|record| &record.metadata);
    [
        ("end_user", labels.end_user.as_deref()),
        (
            "user",
            metadata.and_then(|fields| fields.user_api_key_user_id.as_deref()),
        ),
        (
            "hashed_api_key",
            metadata.and_then(|fields| fields.user_api_key_hash.as_deref()),
        ),
        (
            "api_key_alias",
            metadata.and_then(|fields| fields.user_api_key_alias.as_deref()),
        ),
        (
            "team",
            metadata.and_then(|fields| fields.user_api_key_team_id.as_deref()),
        ),
        (
            "team_alias",
            metadata.and_then(|fields| fields.user_api_key_team_alias.as_deref()),
        ),
        ("model", Some(labels.model)),
        (
            "requested_model",
            labels
                .payload
                .and_then(|record| record.model_group.as_deref()),
        ),
        (
            "api_provider",
            labels
                .payload
                .and_then(|record| record.custom_llm_provider.as_deref()),
        ),
    ]
    .into_iter()
    .filter_map(|(name, value)| {
        value.map(|text| (name.to_owned(), LabelDumpValue::String(text.to_owned())))
    })
    .collect()
}

fn success_entries(entries: &mut Vec<(String, LabelDumpValue)>, labels: &EventLabels<'_>) {
    let Some(record) = labels.payload else {
        return;
    };
    let metadata = &record.metadata;
    push_text(entries, "model_id", record.model_id.as_deref());
    push_text(entries, "model_group", record.model_group.as_deref());
    push_text(entries, "litellm_model_name", Some(labels.model));
    push_text(entries, "status_code", Some("200"));
    push_text(
        entries,
        "route",
        metadata.user_api_key_request_route.as_deref(),
    );
    push_text(
        entries,
        "client_ip",
        metadata.requester_ip_address.as_deref(),
    );
    push_text(entries, "user_agent", metadata.user_agent.as_deref());
    push_text(
        entries,
        "user_email",
        metadata.user_api_key_user_email.as_deref(),
    );
    push_text(entries, "org_id", metadata.user_api_key_org_id.as_deref());
    push_text(
        entries,
        "org_alias",
        metadata.user_api_key_org_alias.as_deref(),
    );
    push_text(entries, "service_tier", service_tier(record));
    push_text(entries, "api_base", Some(record.api_base.as_str()));
    if labels.settings.emit_stream_label {
        entries.push((
            "stream".to_owned(),
            LabelDumpValue::String(payload_stream_text(record)),
        ));
    }
    entries.push((
        "tags".to_owned(),
        LabelDumpValue::Strings(text_tags(&record.request_tags)),
    ));
}

fn push_text(entries: &mut Vec<(String, LabelDumpValue)>, name: &str, value: Option<&str>) {
    if let Some(text) = value {
        entries.push((name.to_owned(), LabelDumpValue::String(text.to_owned())));
    }
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
        class_name: info.error_class.clone().flatten().unwrap_or_default(),
        class: Class::default(),
        message: info.error_message.clone().flatten(),
        status_code: info.error_code.clone().flatten(),
        code: None,
        llm_provider: info.llm_provider.clone().flatten(),
        category: info.error_rate_limit_category.clone().flatten(),
        rate_limit_type: info.error_rate_limit_type.clone().flatten(),
        legacy_class_name: None,
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
