use std::sync::Arc;

use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache, Ttl};
use betterlite_core::api::responses_codex_tools::records::AttributeValue;
use betterlite_core::api::responses_tool_results::input_call::input_function_call;
use betterlite_core::api::responses_tool_results::outbound_call::{
    ApplyPatchOperation, CallIdentity, FunctionToolCall, apply_patch_tool_call,
};
use betterlite_core::api::responses_tool_results::records::ResultMessage;
use betterlite_core::api::responses_tool_results::tool_output::tool_call_output_messages;
use betterlite_models::decisions::Json;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::OrHelperError as _;

#[derive(Debug, Deserialize)]
pub(crate) struct CacheEntry {
    key: String,
    value: CacheValue,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CachedCall {
    item: IndexMap<String, Json>,
    cache: Vec<CacheEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", content = "fields", rename_all = "snake_case")]
pub(crate) enum Provider {
    Plain(IndexMap<String, Json>),
    Record(IndexMap<String, Json>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct Identity {
    id: Option<String>,
    call_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Function {
    name: String,
    arguments: String,
    id: Option<String>,
    call_id: Option<String>,
    provider: Option<Provider>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Patch {
    call_id: String,
    operation: ApplyPatchOperation,
    index: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IdentityCase {
    identities: Vec<Identity>,
    functions: Vec<Function>,
    patches: Vec<Patch>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Operation {
    ToolOutputContent { items: Vec<IndexMap<String, Json>> },
    CachedCallOutput { calls: Vec<CachedCall> },
    InputFunctionCall { items: Vec<IndexMap<String, Json>> },
    OutputCallIdentity(IdentityCase),
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Operation;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Operation::ToolOutputContent { items } => {
                let cache = new_cache(&[]).or_value_error()?;
                let converted = items
                    .iter()
                    .map(|item| tool_call_output_messages(item, &cache))
                    .collect::<Result<Vec<_>, _>>()
                    .or_value_error()?;
                Ok(json!(converted))
            }
            Operation::CachedCallOutput { calls } => {
                let converted = calls
                    .iter()
                    .map(|call| {
                        let cache = new_cache(&call.cache).or_value_error()?;
                        tool_call_output_messages(&call.item, &cache).or_value_error()
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(json!(converted))
            }
            Operation::InputFunctionCall { items } => {
                let converted = items
                    .iter()
                    .map(input_function_call)
                    .collect::<Result<Vec<_>, _>>()
                    .or_value_error()?;
                Ok(json!(converted))
            }
            Operation::OutputCallIdentity(case) => output_call_identity(case),
        }
    }
}

fn new_cache(entries: &[CacheEntry]) -> Result<DualLocalCache, RedactionError> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
    let diagnostics = Diagnostics {
        redactor: Arc::new(redactor),
        emit: Arc::new(|_event| {}),
    };
    let cache = DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        diagnostics,
    );
    for entry in entries {
        cache.set(&entry.key, entry.value.clone(), Ttl::Absent);
    }
    Ok(cache)
}

fn output_call_identity(case: &IdentityCase) -> crate::helpers::HelperResult {
    let identity = CallIdentity::new().or_value_error()?;
    let ids: Vec<String> = case
        .identities
        .iter()
        .map(|entry| identity.tool_call_id(entry.id.as_deref(), entry.call_id.as_deref()))
        .collect();
    let functions = case
        .functions
        .iter()
        .map(|function| {
            let provider = function.provider.as_ref().map(|provider| match provider {
                Provider::Plain(fields) => AttributeValue::Plain(Json::Object(fields.clone())),
                Provider::Record(fields) => AttributeValue::Record(
                    fields
                        .iter()
                        .map(|(name, value)| {
                            Json::List(vec![Json::Text(name.clone()), value.clone()])
                        })
                        .collect(),
                ),
            });
            identity.function_tool_call(&FunctionToolCall {
                name: &function.name,
                arguments: &function.arguments,
                id: function.id.as_deref(),
                call_id: function.call_id.as_deref(),
                provider_specific_fields: provider.as_ref(),
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    let patches: Vec<_> = case
        .patches
        .iter()
        .map(|patch| apply_patch_tool_call(&patch.call_id, &patch.operation, patch.index))
        .collect();
    Ok(json!({"ids": ids, "functions": functions, "patches": patches}))
}
