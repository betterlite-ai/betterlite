#![expect(
    clippy::disallowed_types,
    reason = "raw Responses input items and the chat messages converted from them are provider JSON that betterlite does not model"
)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "serde_json test fixtures built with json! are concise and type-directed"
    )
)]

pub mod error;
pub mod patch;
mod scan;
pub mod texts;
pub mod tools;
pub mod write_back;

pub(super) mod tests;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_input_messages::{InputRequest, input_messages};
use betterlite_core::api::responses_tool_namespaces::values::Fields;
use betterlite_guardrails::generic_guardrail_contracts::{
    GuardrailInputType, GuardrailMessage, GuardrailToolParam,
};
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_message_scope::scoped_indices::{
    ScanScope, ScopeFlags, message_role, role_out_of_guardrail_scope,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use error::RequestRewriteError;

// What a Python guardrail object carries on itself, which the contract trait does not expose
pub struct RequestContext<'ctx> {
    pub skip_system_message: bool,
    pub guardrail_name: Option<&'ctx str>,
    pub limits: ApiLimits,
    pub cache: &'ctx DualLocalCache,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExtractedInputs {
    pub inputs: GuardrailApiInputs,
    pub task_mappings: Vec<(usize, Option<usize>)>,
    pub instructions: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestFields {
    pub input: Vec<Value>,
    pub instructions: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScopedRequest {
    pub structured_messages: Vec<Value>,
    pub scoped_indices: Vec<usize>,
    pub scoped_messages: Option<Vec<GuardrailMessage>>,
}

struct Collected {
    texts: Vec<String>,
    images: Vec<String>,
    task_mappings: Vec<(usize, Option<usize>)>,
}

// The one place this module moves between JSON values and the typed shapes around it
fn serde_convert<Source, Target>(
    value: &Source,
    target: &'static str,
) -> Result<Target, RequestRewriteError>
where
    Source: Serialize + ?Sized,
    Target: DeserializeOwned,
{
    let json = serde_json::to_value(value)
        .map_err(|source| RequestRewriteError::Convert { target, source })?;
    serde_json::from_value(json).map_err(|source| RequestRewriteError::Convert { target, source })
}

pub fn get_structured_messages(
    data: &Object,
    context: &RequestContext<'_>,
) -> Result<Option<Vec<Value>>, RequestRewriteError> {
    let Some(input) = data
        .get("input")
        .filter(|input| **input != GuardrailValue::Null)
    else {
        return Ok(None);
    };
    let instructions = match data.get("instructions") {
        None | Some(GuardrailValue::Null) => None,
        Some(GuardrailValue::Text(text)) => Some(text.clone()),
        Some(_) => {
            return Err(RequestRewriteError::FieldType {
                field: "instructions",
            });
        }
    };
    let request = InputRequest {
        input: Some(serde_convert(input, "input")?),
        instructions,
    };
    let messages = input_messages(&request, false, context.limits, context.cache)?;
    Ok((!messages.is_empty()).then_some(messages))
}

#[must_use]
pub fn scannable_instructions(data: &Object, skip_system: bool) -> Option<&str> {
    match data.get("instructions") {
        Some(GuardrailValue::Text(text)) if !text.is_empty() && !skip_system => Some(text.as_str()),
        Some(_) | None => None,
    }
}

impl Collected {
    fn collect_entry(&mut self, fields: &Object, message_index: usize, content_index: usize) {
        if let Some(GuardrailValue::Text(text)) = fields.get("text") {
            self.texts.push(text.clone());
            self.task_mappings
                .push((message_index, Some(content_index)));
        }
        if matches!(fields.get("type"), Some(GuardrailValue::Text(kind)) if kind == "image_url")
            && let Some(GuardrailValue::Object(image)) = fields.get("image_url")
            && let Some(GuardrailValue::Text(url)) = image.get("url")
            && !url.is_empty()
        {
            self.images.push(url.clone());
        }
    }

    fn collect_message(&mut self, message: &Object, message_index: usize) {
        match message.get("content") {
            Some(GuardrailValue::Text(text)) => {
                self.texts.push(text.clone());
                self.task_mappings.push((message_index, None));
            }
            Some(GuardrailValue::Array(entries)) => {
                for (content_index, entry) in entries.iter().enumerate() {
                    if let GuardrailValue::Object(fields) = entry {
                        self.collect_entry(fields, message_index, content_index);
                    }
                }
            }
            Some(_) | None => {}
        }
    }

    fn collect_input(&mut self, input_data: &GuardrailValue, skip_system: bool) {
        let flags = ScopeFlags {
            skip_system_message: skip_system,
            skip_tool_message: false,
            scan: ScanScope::AllMessages,
        };
        match input_data {
            GuardrailValue::Text(text) => self.texts.push(text.clone()),
            GuardrailValue::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    if let GuardrailValue::Object(message) = item
                        && !role_out_of_guardrail_scope(&message_role(item), &flags)
                    {
                        self.collect_message(message, index);
                    }
                }
            }
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Object(_) => {}
        }
    }
}

pub fn extract_guardrail_inputs(
    data: &Object,
    input_data: &GuardrailValue,
    flattened_tool_groups: &[Vec<Fields>],
    skip_system: bool,
) -> Result<ExtractedInputs, RequestRewriteError> {
    let instructions = scannable_instructions(data, skip_system).map(str::to_owned);
    let mut collected = Collected {
        texts: instructions.iter().cloned().collect(),
        images: Vec::new(),
        task_mappings: Vec::new(),
    };
    collected.collect_input(input_data, skip_system);
    let tools = flattened_tool_groups
        .iter()
        .flatten()
        .map(|tool| serde_convert::<_, GuardrailToolParam>(tool, "request tool"))
        .collect::<Result<Vec<_>, _>>()?;
    let model = match data.get("model") {
        Some(GuardrailValue::Text(model)) => Some(model.clone()),
        Some(_) | None => None,
    };
    Ok(ExtractedInputs {
        inputs: GuardrailApiInputs {
            texts: Some(collected.texts),
            images: (!collected.images.is_empty()).then_some(collected.images),
            tools: (!tools.is_empty()).then_some(tools),
            tool_calls: None,
            structured_messages: None,
            model,
            stream_holdback_chars: None,
        },
        task_mappings: collected.task_mappings,
        instructions,
    })
}

pub async fn process_input_messages<G>(
    data: &mut Object,
    guardrail: &G,
    context: &RequestContext<'_>,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<(), RequestRewriteError>
where
    G: GuardrailContract,
{
    let Some(scan) = scan::scan_request(data, context)? else {
        return Ok(());
    };
    let mut inputs = scan.extracted.inputs.clone();
    inputs
        .structured_messages
        .clone_from(&scan.scoped.scoped_messages);
    let answer = guardrail
        .apply_guardrail(inputs, data, GuardrailInputType::Request, logging_obj)
        .await
        .map_err(|source| RequestRewriteError::Guardrail {
            source: Box::new(source),
        })?;
    let covers_full = guardrail.messages_cover_full_request();
    scan::write_answer(data, &scan, &answer, covers_full, context)
}
