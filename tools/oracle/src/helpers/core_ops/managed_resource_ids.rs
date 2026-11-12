use betterlite_core::api::managed_resource_ids::{
    PassthroughManagedIdScope, UNIFIED_ID_PREFIX, UnifiedIdParts, decode_unified_id,
    is_base64_encoded_unified_id, model_id, parse_unified_id, passthrough_provider_scope,
    provider_resource_id, resource_type, target_model_names, unified_uuid,
};
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    provider: Option<ProviderInput>,
    value: Option<Json>,
    prefix: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    ResolveProvider,
    Detect,
    Decode,
    TargetModelNames,
    ResourceType,
    UnifiedUuid,
    ModelId,
    ProviderResourceId,
    Parse,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ProviderInput {
    Name(String),
    LlmProvider { llm_provider: String },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(Value::Array(input.calls.iter().map(run_call).collect()))
    }
}

fn run_call(call: &Call) -> Value {
    if matches!(call.function, Function::ResolveProvider) {
        let name = match &call.provider {
            Some(ProviderInput::Name(name)) => name,
            Some(ProviderInput::LlmProvider { llm_provider }) => llm_provider,
            None => return Value::Null,
        };
        return match passthrough_provider_scope(name) {
            Some(PassthroughManagedIdScope::Openai) => json!("openai"),
            Some(PassthroughManagedIdScope::Azure) => json!("azure"),
            None => Value::Null,
        };
    }
    // LiteLLM's readers guard with isinstance(str), so any other JSON value takes the empty result
    let Some(Json::Text(text)) = &call.value else {
        return match call.function {
            Function::Detect => json!(false),
            Function::TargetModelNames => json!([]),
            _ => Value::Null,
        };
    };
    match call.function {
        Function::Detect => {
            let prefix = call.prefix.as_deref().unwrap_or(UNIFIED_ID_PREFIX);
            is_base64_encoded_unified_id(text, prefix).map_or(json!(false), Value::from)
        }
        Function::Decode => json!(decode_unified_id(text)),
        Function::TargetModelNames => json!(target_model_names(text)),
        Function::ResourceType => json!(resource_type(text)),
        Function::UnifiedUuid => json!(unified_uuid(text)),
        Function::ModelId => json!(model_id(text)),
        Function::ProviderResourceId => json!(provider_resource_id(text)),
        Function::Parse => parse_unified_id(text).map_or(Value::Null, parts_json),
        Function::ResolveProvider => Value::Null,
    }
}

fn parts_json(parts: UnifiedIdParts) -> Value {
    json!({
        "resource_type": parts.resource_type,
        "unified_uuid": parts.unified_uuid,
        "target_model_names": parts.target_model_names,
        "provider_resource_id": parts.provider_resource_id,
        "model_id": parts.model_id,
    })
}
