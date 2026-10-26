use super::GuardrailExtraValue;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HeaderContext {
    pub headers: Option<BTreeMap<String, GuardrailExtraValue>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LiteParamsContext {
    pub metadata: Option<HeaderContext>,
    #[serde(default)]
    pub extra_headers: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ModelCallDetailsContext {
    pub litellm_params: Option<LiteParamsContext>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LoggingObjectContext {
    pub model_call_details: Option<ModelCallDetailsContext>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InboundRequestContext {
    pub proxy_server_request: Option<HeaderContext>,
    pub headers: Option<BTreeMap<String, GuardrailExtraValue>>,
    pub metadata: Option<HeaderContext>,
    pub litellm_metadata: Option<HeaderContext>,
    pub litellm_params: Option<LiteParamsContext>,
    pub logging_obj: Option<LoggingObjectContext>,
}

#[must_use]
pub fn sanitize_inbound_headers(
    headers: &BTreeMap<String, GuardrailExtraValue>,
    extra_allowlist: Option<&[String]>,
) -> BTreeMap<String, String> {
    let mut sanitized = BTreeMap::new();
    for (name, raw_value) in headers {
        if header_value_allowed(name, extra_allowlist) {
            if let Some(coerced_value) = coerce_header_value(raw_value) {
                sanitized.insert(name.clone(), coerced_value);
            }
        } else {
            sanitized.insert(name.clone(), "[present]".to_owned());
        }
    }
    sanitized
}

fn header_value_allowed(name: &str, extra_allowlist: Option<&[String]>) -> bool {
    let lower_name = name.to_ascii_lowercase();
    let fixed = matches!(
        lower_name.as_str(),
        "host"
            | "accept-encoding"
            | "connection"
            | "accept"
            | "content-type"
            | "user-agent"
            | "content-length"
    );
    let generic = lower_name.starts_with("x-stainless-") || lower_name.starts_with("x-litellm-");
    let extra = extra_allowlist.is_some_and(|names| {
        names
            .iter()
            .any(|allowed_name| allowed_name.eq_ignore_ascii_case(name))
    });
    fixed || generic || extra
}

#[must_use]
pub fn extract_inbound_headers(
    request_data: &InboundRequestContext,
    extra_allowlist: Option<&[String]>,
) -> Option<BTreeMap<String, String>> {
    let combined_allowlist = combined_extra_allowlist(request_data, extra_allowlist);
    let locations = [
        request_data
            .proxy_server_request
            .as_ref()
            .and_then(|context| context.headers.as_ref()),
        request_data.headers.as_ref(),
        request_data
            .metadata
            .as_ref()
            .and_then(|context| context.headers.as_ref()),
        request_data
            .litellm_metadata
            .as_ref()
            .and_then(|context| context.headers.as_ref()),
        request_data
            .litellm_params
            .as_ref()
            .and_then(|params| params.metadata.as_ref())
            .and_then(|context| context.headers.as_ref()),
        logging_headers(request_data),
    ];
    let allowlist = if combined_allowlist.is_empty() {
        extra_allowlist
    } else {
        Some(combined_allowlist.as_slice())
    };
    locations.into_iter().flatten().find_map(|headers| {
        if headers.is_empty() {
            None
        } else {
            Some(sanitize_inbound_headers(headers, allowlist))
        }
    })
}

fn combined_extra_allowlist(
    request_data: &InboundRequestContext,
    extra_allowlist: Option<&[String]>,
) -> Vec<String> {
    let mut combined = extra_allowlist.map_or_else(Vec::new, <[String]>::to_vec);
    let nested_lists = [
        request_data
            .litellm_params
            .as_ref()
            .and_then(|params| params.extra_headers.as_ref()),
        request_data
            .logging_obj
            .as_ref()
            .and_then(|logging| logging.model_call_details.as_ref())
            .and_then(|details| details.litellm_params.as_ref())
            .and_then(|params| params.extra_headers.as_ref()),
    ];
    for nested in nested_lists.into_iter().flatten() {
        combined.extend(nested.iter().cloned());
    }
    combined
}

fn logging_headers(
    request_data: &InboundRequestContext,
) -> Option<&BTreeMap<String, GuardrailExtraValue>> {
    let context = request_data
        .logging_obj
        .as_ref()
        .and_then(|logging| logging.model_call_details.as_ref())
        .and_then(|details| details.litellm_params.as_ref())
        .and_then(|params| params.metadata.as_ref())?;
    context.headers.as_ref()
}

fn coerce_header_value(raw_value: &GuardrailExtraValue) -> Option<String> {
    let text = match raw_value {
        GuardrailExtraValue::Null => "None".to_owned(),
        GuardrailExtraValue::Boolean(boolean) => {
            if *boolean {
                "True".to_owned()
            } else {
                "False".to_owned()
            }
        }
        GuardrailExtraValue::SignedInteger(number) => number.to_string(),
        GuardrailExtraValue::UnsignedInteger(number) => number.to_string(),
        GuardrailExtraValue::Decimal(number) => number.to_string(),
        GuardrailExtraValue::Text(text) => text.clone(),
        GuardrailExtraValue::Array(_) | GuardrailExtraValue::Object(_) => return None,
    };
    Some(text)
}
