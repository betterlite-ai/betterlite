use betterlite_prompts::arguments::json::{Json, Object};

use super::error::SapChatError;
use crate::sap_models::validate_chat_completion_tool;
use crate::sap_models::validate_response_format;

const OPTIONAL_MODULES: [&str; 4] = ["grounding", "masking", "filtering", "translation"];

type PromptParts = (Json, Vec<Json>, Option<Json>, Option<Object>);

fn prompt_parts(model_name: &str, params: &mut Object) -> Result<PromptParts, SapChatError> {
    // SAP AI Core rejects strict as a model param for GPT models, while Anthropic models accept it
    if model_name.starts_with("gpt") {
        params.shift_remove("strict");
    }
    let version = match params.shift_remove("model_version") {
        None | Some(Json::Null) => Json::Text(String::from("latest")),
        Some(given) => given,
    };
    let tools = match params.shift_remove("tools") {
        None | Some(Json::Null) => Vec::new(),
        Some(Json::List(items)) => items
            .iter()
            .map(validate_chat_completion_tool)
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(SapChatError::Field("`tools` must be a list")),
    };
    let format = match params.shift_remove("response_format") {
        None | Some(Json::Null) => None,
        Some(given @ Json::Object(_)) => response_format(&given)?,
        Some(_) => return Err(SapChatError::Field("`response_format` must be an object")),
    };
    let defaults = match params.shift_remove("placeholder_defaults") {
        None | Some(Json::Null) => None,
        Some(Json::Object(fields)) => (!fields.is_empty()).then_some(fields),
        Some(_) => {
            return Err(SapChatError::Field(
                "`placeholder_defaults` must be an object",
            ));
        }
    };
    Ok((version, tools, format, defaults))
}

pub(crate) fn build_prompt_module(
    model_name: &str,
    template: &[Json],
    mut params: Object,
) -> Result<Json, SapChatError> {
    let (version, tools, format, defaults) = prompt_parts(model_name, &mut params)?;
    let mut optional = Object::new();
    for name in OPTIONAL_MODULES {
        let moves = params
            .get(name)
            .is_some_and(|given| !matches!(given, Json::Null));
        if moves && let Some(given) = params.shift_remove(name) {
            optional.insert(name.to_owned(), given);
        }
    }
    let prompt = prompt_payload(template, tools, format, defaults);
    let model = Object::from([
        (String::from("name"), Json::Text(model_name.to_owned())),
        (String::from("params"), Json::Object(params)),
        (String::from("version"), version),
    ]);
    let mut module = Object::from([(
        String::from("prompt_templating"),
        Json::Object(Object::from([
            (String::from("prompt"), Json::Object(prompt)),
            (String::from("model"), Json::Object(model)),
        ])),
    )]);
    module.extend(optional);
    Ok(Json::Object(module))
}

fn prompt_payload(
    template: &[Json],
    tools: Vec<Json>,
    format: Option<Json>,
    defaults: Option<Object>,
) -> Object {
    let mut prompt = Object::new();
    prompt.insert(String::from("template"), Json::List(template.to_vec()));
    if let Some(fields) = defaults {
        prompt.insert(String::from("defaults"), Json::Object(fields));
    }
    if !tools.is_empty() {
        prompt.insert(String::from("tools"), Json::List(tools));
    }
    if let Some(validated) = format {
        prompt.insert(String::from("response_format"), validated);
    }
    prompt
}

fn response_format(value: &Json) -> Result<Option<Json>, SapChatError> {
    let kind = value.as_object().and_then(|fields| fields.get("type"));
    match kind {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) if text.is_empty() => Ok(None),
        Some(_) => Ok(Some(validate_response_format(value)?)),
    }
}
