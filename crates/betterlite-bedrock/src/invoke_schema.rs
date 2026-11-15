use std::future::Future;

use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_prompts::arguments::json::{Json, Object};

const CUSTOM: &str = "custom";
const DEFER_LOADING: &str = "defer_loading";
const EAGER_INPUT_STREAMING: &str = "eager_input_streaming";
const FORMAT: &str = "format";
const MESSAGES: &str = "messages";
const NAME: &str = "name";
const NATIVE_STRUCTURED_OUTPUT: &str = "supports_native_structured_output";
const OUTPUT_CONFIG: &str = "output_config";
const OUTPUT_CONFIG_FLAG: &str = "supports_output_config";
const OUTPUT_FORMAT: &str = "output_format";
const ROLE: &str = "role";
const SCHEMA: &str = "schema";
const TOOLS: &str = "tools";
const TYPE: &str = "type";
const UNNAMED_TOOL_PREFIX: &str = "litellm_unnamed_tool_";
const USER: &str = "user";

// The composing layer answers these through B2221's supports_factory and B569's model_supports_effort_param.
pub trait InvokeSchemaCapabilities {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send;

    fn supports_effort_param(&mut self, model: &str) -> impl Future<Output = bool> + Send;
}

#[must_use]
pub fn pop_output_config_format(body: &mut Object) -> Option<Object> {
    let output_format = match body.get_mut(OUTPUT_CONFIG) {
        Some(Json::Object(fields)) => fields.shift_remove(FORMAT),
        _ => return None,
    };
    let emptied = body
        .get(OUTPUT_CONFIG)
        .is_some_and(|value| matches!(value, Json::Object(fields) if fields.is_empty()));
    if emptied {
        body.shift_remove(OUTPUT_CONFIG);
    }
    match output_format {
        Some(Json::Object(fields)) => Some(fields),
        _ => None,
    }
}

pub fn inline_output_format(output_format: &Object, body: &mut Object) {
    let Some(schema) = output_format
        .get(SCHEMA)
        .filter(|candidate| candidate.is_truthy())
    else {
        return;
    };
    let Some(Json::List(mut messages)) = body.get(MESSAGES).cloned() else {
        return;
    };
    let Some(index) = messages.iter().rposition(|message| {
        matches!(
            message,
            Json::Object(fields)
                if matches!(fields.get(ROLE), Some(Json::Text(role)) if role == USER)
        )
    }) else {
        return;
    };
    let Some(Json::Object(mut original)) = messages.get(index).cloned() else {
        return;
    };
    let Ok(text) = serde_json::to_string(schema) else {
        return;
    };
    let mut content = match original.get("content") {
        None => Vec::new(),
        Some(Json::Text(existing)) => vec![text_block(existing)],
        Some(Json::List(items)) => items.clone(),
        Some(_) => return,
    };
    content.push(text_block(&text));
    original.insert("content".to_owned(), Json::List(content));
    let Some(slot) = messages.get_mut(index) else {
        return;
    };
    *slot = Json::Object(original);
    body.insert(MESSAGES.to_owned(), Json::List(messages));
}

fn text_block(text: &str) -> Json {
    let mut fields = Object::new();
    fields.insert(TYPE.to_owned(), Json::Text("text".to_owned()));
    fields.insert("text".to_owned(), Json::Text(text.to_owned()));
    Json::Object(fields)
}

async fn bedrock_model_supports<Capabilities>(
    capabilities: &mut Capabilities,
    model: &str,
    key: &str,
) -> bool
where
    Capabilities: InvokeSchemaCapabilities,
{
    let request = CapabilityRequest {
        model,
        custom_llm_provider: Some("bedrock"),
    };
    capabilities.supports_factory(&request, key).await
}

pub async fn apply_structured_output<Capabilities>(
    model: &str,
    body: &mut Object,
    capabilities: &mut Capabilities,
) where
    Capabilities: InvokeSchemaCapabilities,
{
    let legacy = body.shift_remove(OUTPUT_FORMAT);
    let nested = pop_output_config_format(body);
    let schema_format = match legacy {
        Some(Json::Object(fields)) => Some(fields),
        _ => nested,
    };
    let Some(selected_format) = schema_format else {
        return;
    };
    if bedrock_model_supports(capabilities, model, NATIVE_STRUCTURED_OUTPUT).await {
        let mut output_config = match body.shift_remove(OUTPUT_CONFIG) {
            Some(Json::Object(fields)) => fields,
            _ => Object::new(),
        };
        output_config.insert(FORMAT.to_owned(), Json::Object(selected_format));
        body.insert(OUTPUT_CONFIG.to_owned(), Json::Object(output_config));
        return;
    }
    tracing::warn!(
        "Bedrock Invoke: model={model} does not advertise `supports_native_structured_output` in model_prices_and_context_window.json, so the JSON schema was inlined into the last user message and is NOT enforced by the model."
    );
    inline_output_format(&selected_format, body);
}

pub async fn strip_unsupported_config_keys<Capabilities>(
    model: &str,
    body: &mut Object,
    capabilities: &mut Capabilities,
) where
    Capabilities: InvokeSchemaCapabilities,
{
    let Some(Json::Object(output_config)) = body.get(OUTPUT_CONFIG).cloned() else {
        return;
    };
    if output_config.keys().all(|key| key == FORMAT) {
        return;
    }
    let supported = bedrock_model_supports(capabilities, model, OUTPUT_CONFIG_FLAG).await
        || capabilities.supports_effort_param(model).await;
    if supported {
        return;
    }
    tracing::warn!(
        "Bedrock Invoke: stripping unsupported `output_config` keys for model={model}: neither `supports_output_config` nor any `supports_*_reasoning_effort` flag is set in model_prices_and_context_window.json. Add the capability flag to the model JSON entry if this model accepts `output_config`."
    );
    match output_config.get(FORMAT) {
        None | Some(Json::Null) => {
            body.shift_remove(OUTPUT_CONFIG);
        }
        Some(format) => {
            let mut fields = Object::new();
            fields.insert(FORMAT.to_owned(), format.clone());
            body.insert(OUTPUT_CONFIG.to_owned(), Json::Object(fields));
        }
    }
}

pub fn normalize_tool_custom_field(body: &mut Object) {
    for_each_tool_object(body, |_, fields| {
        let defer_loading = match fields.shift_remove(CUSTOM) {
            Some(Json::Object(mut custom)) => custom.shift_remove(DEFER_LOADING),
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
            | None => None,
        };
        if let Some(Json::Bool(deferred)) = defer_loading {
            fields
                .entry(DEFER_LOADING.to_owned())
                .or_insert(Json::Bool(deferred));
        }
    });
}

#[must_use]
pub fn strip_eager_input_streaming(body: &Object) -> Option<Vec<Object>> {
    let Some(Json::List(tools)) = body.get(TOOLS) else {
        return None;
    };
    let mut stripped = Vec::with_capacity(tools.len());
    for tool in tools {
        let Json::Object(fields) = tool else {
            return None;
        };
        let mut kept = fields.clone();
        kept.shift_remove(EAGER_INPUT_STREAMING);
        stripped.push(kept);
    }
    Some(stripped)
}

pub fn normalize_schema_custom_types(schema: &mut Json) {
    let Json::Object(fields) = schema else {
        return;
    };
    if matches!(fields.get(TYPE), Some(Json::Text(kind)) if kind == "custom") {
        fields.insert(TYPE.to_owned(), Json::Text("object".to_owned()));
    }
    // the JSON decoder caps nesting, so this walk cannot exhaust the stack
    for key in ["items", "additionalProperties"] {
        if let Some(child) = fields.get_mut(key) {
            normalize_schema_custom_types(child);
        }
    }
    if let Some(Json::Object(properties)) = fields.get_mut("properties") {
        for child in properties.values_mut() {
            normalize_schema_custom_types(child);
        }
    }
    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(Json::List(members)) = fields.get_mut(key) {
            for child in members {
                normalize_schema_custom_types(child);
            }
        }
    }
}

fn for_each_tool_object<Apply>(body: &mut Object, mut apply: Apply)
where
    Apply: FnMut(usize, &mut Object),
{
    let Some(Json::List(tools)) = body.get_mut(TOOLS) else {
        return;
    };
    for (index, tool) in tools.iter_mut().enumerate() {
        if let Json::Object(fields) = tool {
            apply(index, fields);
        }
    }
}

pub fn normalize_tool_schema_types(body: &mut Object) {
    for_each_tool_object(body, |_, fields| {
        if let Some(schema) = fields.get_mut("input_schema") {
            normalize_schema_custom_types(schema);
        }
    });
}

pub fn ensure_tool_names(body: &mut Object) {
    let Some(Json::List(tools)) = body.get_mut(TOOLS) else {
        return;
    };
    let mut unnamed = Vec::new();
    for (index, tool) in tools.iter().enumerate() {
        let Json::Object(fields) = tool else {
            continue;
        };
        // Python's str.strip also trims the separators U+001C to U+001F
        let blank = fields.get(NAME).is_none_or(|name| match name {
            Json::Null => true,
            Json::Text(text) => text.chars().all(|character| {
                character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
            }),
            Json::Bool(_) | Json::Number(_) | Json::Object(_) | Json::List(_) => false,
        });
        if blank {
            unnamed.push(index);
        }
    }
    for index in unnamed {
        let Some(Json::Object(fields)) = tools.get_mut(index) else {
            continue;
        };
        let name = format!("{UNNAMED_TOOL_PREFIX}{index}");
        fields.insert(NAME.to_owned(), Json::Text(name));
    }
}

mod tests;
