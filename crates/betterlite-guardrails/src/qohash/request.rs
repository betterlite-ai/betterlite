use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::{LITELLM_VERSION, QostodianNexus};
use crate::generic_guardrail_api::extract_user_api_key_metadata;
use crate::generic_guardrail_contracts::headers::{
    HeaderContext, InboundRequestContext, LiteParamsContext, LoggingObjectContext,
    ModelCallDetailsContext, extract_inbound_headers,
};
use crate::generic_guardrail_contracts::{GenericGuardrailAPIRequest, GuardrailInputType};
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};

impl QostodianNexus {
    pub(super) fn request_for(
        &self,
        inputs: &GuardrailApiInputs,
        request_data: &BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&GuardrailLogTarget>,
    ) -> GenericGuardrailAPIRequest {
        let (litellm_call_id, litellm_trace_id) = call_and_trace_ids(logging_obj);
        // metadata values are strings in LiteLLM's TypedDict, so a value of another type names no field it reads
        let litellm_metadata = metadata_strings(request_data.get("litellm_metadata"));
        let metadata = metadata_strings(request_data.get("metadata"));
        GenericGuardrailAPIRequest {
            input_type,
            litellm_call_id,
            litellm_trace_id,
            structured_messages: inputs.structured_messages.clone(),
            images: inputs.images.clone(),
            tools: inputs.tools.clone(),
            texts: inputs.texts.clone(),
            request_data: extract_user_api_key_metadata(&litellm_metadata, &metadata),
            request_headers: extract_inbound_headers(
                &inbound_context(request_data, logging_obj),
                Some(self.extra_headers.as_slice()),
            ),
            litellm_version: Some(LITELLM_VERSION.to_owned()),
            provider_params: None,
            tool_calls: inputs.tool_calls.clone(),
            model: inputs.model.clone(),
        }
    }
}

fn object_at<'data>(
    data: &'data BTreeMap<String, GuardrailValue>,
    name: &str,
) -> Option<&'data BTreeMap<String, GuardrailValue>> {
    match data.get(name) {
        Some(GuardrailValue::Object(entries)) => Some(entries),
        _ => None,
    }
}

fn header_context(context: &BTreeMap<String, GuardrailValue>) -> HeaderContext {
    match object_at(context, "headers") {
        Some(headers) => HeaderContext {
            headers: Some(headers.clone()),
        },
        None => HeaderContext { headers: None },
    }
}

fn text_array(value: Option<&GuardrailValue>) -> Option<Vec<String>> {
    let GuardrailValue::Array(items) = value? else {
        return None;
    };
    let mut names = Vec::new();
    for item in items {
        if let GuardrailValue::Text(name) = item {
            names.push(name.clone());
        }
    }
    (!names.is_empty()).then_some(names)
}

fn lite_params_context(
    params: Option<&BTreeMap<String, GuardrailValue>>,
) -> Option<LiteParamsContext> {
    let entries = params?;
    Some(LiteParamsContext {
        metadata: object_at(entries, "metadata").map(header_context),
        extra_headers: text_array(entries.get("extra_headers")),
    })
}

fn logging_context(logging_obj: Option<&GuardrailLogTarget>) -> Option<LoggingObjectContext> {
    let params = logging_obj?.litellm_params.as_ref()?;
    Some(LoggingObjectContext {
        model_call_details: Some(ModelCallDetailsContext {
            litellm_params: lite_params_context(Some(params)),
        }),
    })
}

fn inbound_context(
    request_data: &BTreeMap<String, GuardrailValue>,
    logging_obj: Option<&GuardrailLogTarget>,
) -> InboundRequestContext {
    InboundRequestContext {
        proxy_server_request: object_at(request_data, "proxy_server_request").map(header_context),
        headers: object_at(request_data, "headers").cloned(),
        metadata: object_at(request_data, "metadata").map(header_context),
        litellm_metadata: object_at(request_data, "litellm_metadata").map(header_context),
        litellm_params: lite_params_context(object_at(request_data, "litellm_params")),
        logging_obj: logging_context(logging_obj),
    }
}

fn metadata_strings(value: Option<&GuardrailValue>) -> BTreeMap<String, Option<String>> {
    let mut entries = BTreeMap::new();
    if let Some(GuardrailValue::Object(values)) = value {
        for (name, entry) in values {
            match entry {
                GuardrailValue::Text(text) => {
                    entries.insert(name.clone(), Some(text.clone()));
                }
                GuardrailValue::Null => {
                    entries.insert(name.clone(), None);
                }
                GuardrailValue::Boolean(_)
                | GuardrailValue::SignedInteger(_)
                | GuardrailValue::UnsignedInteger(_)
                | GuardrailValue::Decimal(_)
                | GuardrailValue::Array(_)
                | GuardrailValue::Object(_) => {}
            }
        }
    }
    entries
}

fn call_and_trace_ids(
    logging_obj: Option<&GuardrailLogTarget>,
) -> (Option<String>, Option<String>) {
    let Some(params) = logging_obj.and_then(|target| target.litellm_params.as_ref()) else {
        return (None, None);
    };
    let field = |name: &str| match params.get(name) {
        Some(GuardrailValue::Text(value)) => Some(value.clone()),
        _ => None,
    };
    (field("litellm_call_id"), field("litellm_trace_id"))
}
