use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object as Params};
use betterlite_prompts::builtin::Message;
use betterlite_provider::petals::config::map_openai_params;
use betterlite_provider::petals::request::{CustomPrompt, RequestBody, RequestContext};
use chrono::DateTime;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use crate::helpers::{HelperError, HelperResult};

use super::{FIXED_NOW, petals_config, prompts, runtime};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestInput {
    #[serde(default)]
    custom_prompt_dict: BTreeMap<String, CustomPrompt>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    requests: Vec<RequestCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestCase {
    model: String,
    messages: Vec<Message>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    max_tokens: Option<Json>,
    #[serde(default)]
    temperature: Option<Json>,
    #[serde(default)]
    top_p: Option<Json>,
    #[serde(default)]
    stream: Option<Json>,
}

pub(crate) fn request_values(input: &RequestInput) -> HelperResult {
    let executor = runtime()?;
    let config = petals_config(&input.env)?;
    let prompts = prompts();
    let context = RequestContext {
        config: &config,
        prompts: &prompts,
        custom_prompts: &input.custom_prompt_dict,
        now: DateTime::from_timestamp(FIXED_NOW, 0).unwrap_or_default(),
    };
    let mut bodies: Vec<Value> = Vec::new();
    for request in &input.requests {
        let entry = one_request(&executor, &context, request)?;
        bodies.push(entry);
    }
    Ok(json!({"requests": bodies}))
}

fn one_request(
    executor: &Runtime,
    context: &RequestContext<'_>,
    request: &RequestCase,
) -> HelperResult {
    let mut options = Params::new();
    map_openai_params(&openai_params(request), &mut options);
    match executor.block_on(context.transform_request(
        &request.model,
        &request.messages,
        request.api_base.as_deref(),
        &options,
    )) {
        Ok(body) => Ok(json!({"ok": true, "form": form(&body)?})),
        Err(error) => Ok(json!({"ok": false, "status": error.status()})),
    }
}

fn openai_params(request: &RequestCase) -> Params {
    let mut params = Params::new();
    let fields = [
        ("max_tokens", &request.max_tokens),
        ("temperature", &request.temperature),
        ("top_p", &request.top_p),
        ("stream", &request.stream),
    ];
    for (name, value) in fields {
        if let Some(given) = value {
            params.insert(String::from(name), given.clone());
        }
    }
    params
}

fn form(body: &RequestBody) -> Result<Params, Box<HelperError>> {
    let mut form = Params::new();
    form.insert(String::from("model"), Json::Text(body.model.clone()));
    form.insert(
        String::from("inputs"),
        Json::Text(body.inputs.clone().unwrap_or_default()),
    );
    for (name, value) in &body.parameters {
        let Some(text) = form_text(value) else {
            return Err(Box::new(HelperError {
                class: String::from("ValueError"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: format!("the parameter {name} is not a form value"),
            }));
        };
        form.insert(name.clone(), Json::Text(text));
    }
    Ok(form)
}

fn form_text(value: &Json) -> Option<String> {
    match value {
        Json::Null => Some(String::new()),
        Json::Bool(flag) => Some(flag.to_string()),
        Json::Number(number) => Some(number.to_string()),
        Json::Text(text) => Some(text.clone()),
        Json::List(_) | Json::Object(_) => None,
    }
}
