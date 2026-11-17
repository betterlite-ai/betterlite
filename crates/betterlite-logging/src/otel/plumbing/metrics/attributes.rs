use serde::Serialize;

use crate::otel::legacy_metrics::attributes::CommonAttributes;
use crate::otel::legacy_metrics::error::LegacyMetricsError;
use crate::otel::legacy_metrics::filter::METRIC_METADATA_KEYS;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::vocabulary::constants::attribute_keys::gen_ai;
use crate::otel::vocabulary::resolvers::{resolve_operation, resolve_provider};
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

// sorted, which is the order the ineligible-name warning lists them in
pub const METRIC_ATTRIBUTE_CEILING: [&str; 12] = [
    "gen_ai.framework",
    "gen_ai.operation.name",
    "gen_ai.provider.name",
    "gen_ai.request.model",
    "gen_ai.system",
    "hidden_params",
    "metadata.user_api_key_alias",
    "metadata.user_api_key_hash",
    "metadata.user_api_key_org_id",
    "metadata.user_api_key_team_alias",
    "metadata.user_api_key_team_id",
    "metadata.user_api_key_user_id",
];

// the only hidden_params field that becomes part of the label, since the object as a whole moves per call
pub const BOUNDED_HIDDEN_PARAM_KEYS: [&str; 1] = ["model_id"];

// a call with no provider carries no provider label at all: a placeholder would mint a permanent series
#[must_use]
pub fn provider_attributes(custom_llm_provider: Option<&str>) -> CommonAttributes {
    let Some(label) = custom_llm_provider.filter(|label| !label.is_empty()) else {
        return Vec::new();
    };
    vec![
        (
            gen_ai::PROVIDER_NAME.to_owned(),
            Some(resolve_provider(label).to_owned()),
        ),
        (gen_ai::SYSTEM.to_owned(), Some(label.to_owned())),
    ]
}

pub fn common_attributes(kwargs: &MetadataMap) -> Result<CommonAttributes, LegacyMetricsError> {
    let call_type = kwargs
        .get("call_type")
        .and_then(OtelModelValue::as_str)
        .unwrap_or_default();
    let provider = kwargs
        .get("litellm_params")
        .and_then(OtelModelValue::as_object)
        .and_then(|params| params.get("custom_llm_provider"))
        .and_then(OtelModelValue::as_str);
    let mut attributes: CommonAttributes = vec![(
        gen_ai::OPERATION_NAME.to_owned(),
        Some(resolve_operation(call_type).to_string()),
    )];
    attributes.extend(provider_attributes(provider));
    attributes.push((
        gen_ai::REQUEST_MODEL.to_owned(),
        kwargs
            .get("model")
            .and_then(OtelModelValue::as_str)
            .map(str::to_owned),
    ));
    attributes.push(("gen_ai.framework".to_owned(), Some("litellm".to_owned())));
    let Some(standard) = kwargs
        .get("standard_logging_object")
        .and_then(OtelModelValue::as_object)
    else {
        return Ok(attributes);
    };
    if let Some(metadata) = standard.get("metadata").and_then(OtelModelValue::as_object) {
        attributes.extend(metadata_attributes(metadata)?);
    }
    if let Some(hidden) = bounded_hidden_params(standard)? {
        attributes.push(("hidden_params".to_owned(), Some(hidden)));
    }
    Ok(attributes)
}

pub fn bounded_attributes(kwargs: &MetadataMap) -> Result<CommonAttributes, LegacyMetricsError> {
    Ok(common_attributes(kwargs)?
        .into_iter()
        .filter(|(key, _)| METRIC_ATTRIBUTE_CEILING.contains(&key.as_str()))
        .collect())
}

fn metadata_attributes(metadata: &MetadataMap) -> Result<CommonAttributes, LegacyMetricsError> {
    METRIC_METADATA_KEYS
        .iter()
        .filter_map(|key| metadata.get(*key).map(|value| (*key, value)))
        .filter(|(_, value)| !matches!(value, OtelModelValue::Null(())))
        .map(|(key, value)| {
            let text = match value.as_str() {
                Some(text) => text.to_owned(),
                None => dump(value)?,
            };
            Ok((format!("metadata.{key}"), Some(text)))
        })
        .collect()
}

fn bounded_hidden_params(standard: &MetadataMap) -> Result<Option<String>, LegacyMetricsError> {
    let Some(hidden) = standard
        .get("hidden_params")
        .and_then(OtelModelValue::as_object)
    else {
        return Ok(None);
    };
    let bounded: MetadataMap = BOUNDED_HIDDEN_PARAM_KEYS
        .iter()
        .filter_map(|key| {
            hidden
                .get(*key)
                .map(|value| ((*key).to_owned(), value.clone()))
        })
        .filter(|(_, value)| !matches!(value, OtelModelValue::Null(())))
        .collect();
    if bounded.is_empty() {
        return Ok(None);
    }
    dump(&bounded).map(Some)
}

pub(super) fn dump<T>(value: &T) -> Result<String, LegacyMetricsError>
where
    T: Serialize,
{
    safe_dumps(value, DEFAULT_MAX_RECURSE_DEPTH, None)
        .map_err(|source| LegacyMetricsError::Dump { source })
}
