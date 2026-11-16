use super::error::LegacyMetricsError;
use super::filter::METRIC_METADATA_KEYS;
use crate::otel::integrations::arize_attributes::truthy::is_truthy;
use crate::otel::legacy_policy::semconv_attrs::gen_ai_operation_name;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

pub type CommonAttributes = Vec<(String, Option<String>)>;

// a call with no provider carries no provider label at all
#[must_use]
pub fn provider_label(kwargs: &MetadataMap) -> Option<&str> {
    kwargs
        .get("litellm_params")
        .and_then(OtelModelValue::as_object)
        .and_then(|params| params.get("custom_llm_provider"))
        .and_then(OtelModelValue::as_str)
        .filter(|label| !label.is_empty())
}

pub fn common_attributes(
    kwargs: &MetadataMap,
    semconv_latest: bool,
) -> Result<CommonAttributes, LegacyMetricsError> {
    let call_type = kwargs.get("call_type").and_then(OtelModelValue::as_str);
    let operation = if semconv_latest {
        gen_ai_operation_name(call_type)
    } else {
        "chat"
    };
    let mut attributes: CommonAttributes = vec![
        (
            "gen_ai.operation.name".to_owned(),
            Some(operation.to_owned()),
        ),
        (
            "gen_ai.system".to_owned(),
            provider_label(kwargs).map(str::to_owned),
        ),
        (
            "gen_ai.request.model".to_owned(),
            kwargs
                .get("model")
                .and_then(OtelModelValue::as_str)
                .map(str::to_owned),
        ),
        ("gen_ai.framework".to_owned(), Some("litellm".to_owned())),
    ];
    let standard = kwargs
        .get("standard_logging_object")
        .and_then(OtelModelValue::as_object);
    if let Some(fields) = standard {
        attributes.extend(metadata_attributes(fields)?);
        if let Some(hidden) = fields
            .get("hidden_params")
            .filter(|hidden| is_truthy(hidden))
        {
            attributes.push(("hidden_params".to_owned(), Some(dump(hidden)?)));
        }
    }
    Ok(attributes)
}

fn metadata_attributes(standard: &MetadataMap) -> Result<CommonAttributes, LegacyMetricsError> {
    let Some(metadata) = standard.get("metadata").and_then(OtelModelValue::as_object) else {
        return Ok(Vec::new());
    };
    let mut attributes = CommonAttributes::new();
    for key in METRIC_METADATA_KEYS {
        let text = match metadata.get(key) {
            None | Some(OtelModelValue::Null(())) => continue,
            Some(OtelModelValue::String(text)) => text.clone(),
            Some(value) => dump(value)?,
        };
        attributes.push((format!("metadata.{key}"), Some(text)));
    }
    Ok(attributes)
}

fn dump(value: &OtelModelValue) -> Result<String, LegacyMetricsError> {
    safe_dumps(value, DEFAULT_MAX_RECURSE_DEPTH, None)
        .map_err(|source| LegacyMetricsError::Dump { source })
}
