use std::convert::Infallible;
use std::sync::Mutex;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_chat_options::context::OptionsContext;
use betterlite_core::api::messages_chat_options::error::OptionsError;
use betterlite_core::api::messages_chat_options::metadata::translate_metadata;
use betterlite_core::api::messages_chat_options::passthrough::{
    copy_untranslated_params, translate_stop_sequences,
};
use betterlite_core::api::messages_chat_options::system::{
    insert_system_message, mid_turn_system_message,
};
use betterlite_core::api::messages_chat_options::thinking::translate_thinking;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::projections::reasoning_effort::ThinkingBudgetThresholds;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

// The cases carry a few hundred bytes, so the crate's own tests own the request bounds.
const LIMITS: ApiLimits = ApiLimits {
    max_json_bytes: 16 * 1024 * 1024,
    max_event_bytes: 1024 * 1024,
    max_items: 64 * 1024,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    request: Json,
    custom_llm_provider: Option<String>,
    auto_summary: bool,
    declared: Option<Vec<String>>,
}

struct ProbeCall {
    model: String,
    provider: Option<String>,
}

struct Parameters {
    declared: Option<Vec<String>>,
    calls: Mutex<Vec<ProbeCall>>,
}

impl Parameters {
    const fn new(declared: Option<Vec<String>>) -> Self {
        Self {
            declared,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn into_calls(self) -> Vec<Value> {
        let calls = self
            .calls
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        calls
            .into_iter()
            .map(|call| json!({ "model": call.model, "provider": call.provider }))
            .collect()
    }
}

impl SupportedParameterSource for Parameters {
    type Error = Infallible;

    fn supported_openai_params(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Infallible> {
        let mut calls = self
            .calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        calls.push(ProbeCall {
            model: model.to_owned(),
            provider: custom_llm_provider.map(str::to_owned),
        });
        Ok(self.declared.clone())
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let Json::Object(mut request) = input.request.clone() else {
            return Err(failure(
                "InvalidInput",
                None,
                "request must be an object".to_owned(),
            ));
        };
        let model = request
            .get("model")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_owned();
        let parameters = Parameters::new(input.declared.clone());
        let context = OptionsContext {
            model: &model,
            custom_llm_provider: input.custom_llm_provider.as_deref(),
            auto_summary: input.auto_summary,
            thresholds: ThinkingBudgetThresholds::default(),
            limits: LIMITS,
            parameters: &parameters,
        };
        let mut messages = starting_messages(&request, &model);
        insert_system_message(&mut messages, &request);
        let mut kwargs = Object::new();
        kwargs.insert("model".to_owned(), Json::Text(model.clone()));
        kwargs.insert("messages".to_owned(), Json::List(messages));
        translate_options(&mut request, &mut kwargs, &context)
            .map_err(|error| failure("OptionsError", None, error.to_string()))?;
        let probes = parameters.into_calls();
        Ok(json!({
            "kwargs": Json::Object(kwargs),
            "request": Json::Object(request),
            "probes": probes,
        }))
    }
}

fn translate_options(
    request: &mut Object,
    kwargs: &mut Object,
    context: &OptionsContext<'_>,
) -> Result<(), OptionsError> {
    translate_metadata(request, kwargs, context)?;
    translate_thinking(request, kwargs, context)?;
    translate_stop_sequences(request, kwargs);
    copy_untranslated_params(request, kwargs);
    Ok(())
}

// The fixtures carry plain user text and system entries, so the chat messages are the mid-turn translation plus the rest as sent.
fn starting_messages(request: &Object, model: &str) -> Vec<Json> {
    let Some(Json::List(messages)) = request.get("messages") else {
        return Vec::new();
    };
    let system_model = (!model.is_empty()).then_some(model);
    let mut translated = Vec::with_capacity(messages.len());
    for message in messages {
        let Some(fields) = message.as_object() else {
            continue;
        };
        if fields.get("role").and_then(Json::as_str) == Some("system") {
            if let Some(system) = mid_turn_system_message(fields, system_model) {
                translated.push(Json::Object(system));
            }
            continue;
        }
        translated.push(message.clone());
    }
    translated
}

fn failure(class: &str, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}
