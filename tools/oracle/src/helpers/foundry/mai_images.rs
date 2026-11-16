use betterlite_foundry::mai_images::error::MaiImageError;
use betterlite_foundry::mai_images::params::map_openai_params;
use betterlite_foundry::mai_images::reply::{
    ImageLogging, MaiImageReply, PostCall, transform_image_reply,
};
use betterlite_foundry::mai_images::url::{ImageOperation, is_mai_model, mai_image_url};
use betterlite_foundry::mai_images::usage::normalize_mai_image_usage;
use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde_json::{Map, Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod input;
mod tests;

use input::{Input, OperationName, ParamsCall, ReplyCall, UrlCall};

#[derive(Serialize)]
struct Logged {
    input: String,
    api_key: Option<String>,
    original_response: Object,
    complete_input_dict: Object,
}

#[derive(Default)]
struct Recorder {
    logged: Option<Logged>,
}

impl ImageLogging for Recorder {
    fn post_call(&mut self, call: PostCall<'_>) {
        self.logged = Some(Logged {
            input: call.input.to_owned(),
            api_key: call.api_key.map(|key| key.expose_secret().to_owned()),
            original_response: call.original_response.clone(),
            complete_input_dict: call.complete_input_dict.clone(),
        });
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answer = Map::new();
        if let Some(urls) = &input.urls {
            let outcomes: Vec<Value> = urls.iter().map(url_outcome).collect();
            answer.insert("urls".to_owned(), Value::Array(outcomes));
        }
        if let Some(models) = &input.models {
            let families: Vec<bool> = models.iter().map(|model| is_mai_model(model)).collect();
            answer.insert("models".to_owned(), json!(families));
        }
        if let Some(params) = &input.params {
            let outcomes: Vec<Value> = params.iter().map(params_outcome).collect();
            answer.insert("params".to_owned(), Value::Array(outcomes));
        }
        if let Some(usages) = &input.usages {
            let outcomes: Vec<Value> = usages
                .iter()
                .map(|usage| outcome(normalize_mai_image_usage(usage.as_ref())))
                .collect();
            answer.insert("usages".to_owned(), Value::Array(outcomes));
        }
        if let Some(replies) = &input.replies {
            let outcomes: Vec<Value> = replies.iter().map(reply_outcome).collect();
            answer.insert("replies".to_owned(), Value::Array(outcomes));
        }
        Ok(Value::Object(answer))
    }
}

// The Python class each failure maps to; the reply-shape failures have no LiteLLM counterpart
const fn error_kind(error: &MaiImageError) -> &'static str {
    match error {
        MaiImageError::ApiBaseRequired { .. } => "ValueError",
        MaiImageError::NotJson { .. } => "OpenAIError",
        MaiImageError::UnsupportedSize { .. }
        | MaiImageError::InvalidSize { .. }
        | MaiImageError::SizeBelowMinimum { .. }
        | MaiImageError::SizeAboveMaximum { .. }
        | MaiImageError::CountNotWhole { .. }
        | MaiImageError::CountNotOne { .. }
        | MaiImageError::UnsupportedParam { .. } => "UnsupportedParamsError",
        MaiImageError::NotAnObject
        | MaiImageError::FieldType { .. }
        | MaiImageError::TokenOverflow { .. } => "InvalidReply",
    }
}

fn outcome<Success>(result: Result<Success, MaiImageError>) -> Value
where
    Success: Serialize,
{
    match result {
        Ok(value) => json!({ "value": value }),
        Err(error) => json!({
            "error": {
                "kind": error_kind(&error),
                "status": error.status_code(),
                "provider": error.llm_provider(),
            }
        }),
    }
}

fn url_outcome(call: &UrlCall) -> Value {
    let operation = match call.operation {
        OperationName::Generation => ImageOperation::Generation,
        OperationName::Edit => ImageOperation::Edit,
    };
    outcome(mai_image_url(
        operation,
        call.api_base.as_deref(),
        call.api_version.as_deref(),
    ))
}

fn params_outcome(call: &ParamsCall) -> Value {
    outcome(map_openai_params(
        &call.non_default_params,
        call.optional_params.clone(),
        &call.model,
        call.drop_params,
    ))
}

fn reply_outcome(call: &ReplyCall) -> Value {
    let key = call.api_key.as_deref().map(SecretString::from);
    let mut recorder = Recorder::default();
    let result = transform_image_reply(
        &MaiImageReply {
            body: call.body.as_bytes(),
            status_code: call.status_code,
            request_data: &call.request_data,
            optional_params: &call.optional_params,
            api_key: key.as_ref(),
        },
        &mut recorder,
    );
    outcome(result.map(|response| json!({ "size": response.size, "logged": recorder.logged })))
}
