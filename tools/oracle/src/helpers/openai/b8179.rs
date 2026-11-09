use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::nlp_cloud_chat::NlpCloudConfig;
use betterlite_openai::nlp_cloud_chat::error::{NlpCloudError, get_error_class};
use betterlite_openai::nlp_cloud_chat::headers::validate_environment;
use betterlite_openai::nlp_cloud_chat::params::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use betterlite_openai::nlp_cloud_chat::reply::{ReplyInput, decode_reply};
use betterlite_openai::nlp_cloud_chat::request::{complete_url, transform_request};
use betterlite_openai::nlp_cloud_chat::stream::clean_and_iterate_chunks;
use betterlite_openai::openai_base_config::{Headers, Object};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperResult};

const DEFAULT_REPLY: &str =
    r#"{"generated_text":"ok","nb_input_tokens":0,"nb_generated_tokens":0}"#;
const FAILURE_STATUS: u16 = 500;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    SupportedParams,
    MapParams {
        params: Object,
        #[serde(default)]
        optional_params: Object,
    },
    Headers {
        #[serde(default)]
        headers: Headers,
        api_key: Option<String>,
    },
    ErrorClass {
        message: String,
        status: u16,
        headers: Option<Headers>,
    },
    Defaults {
        defaults: Object,
    },
    Completion(CompletionStep),
    Reply {
        status: u16,
        body: String,
        model: String,
        created: u64,
    },
    StreamLines {
        lines: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompletionStep {
    model: String,
    api_base: String,
    messages: Vec<ChatMessage>,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    defaults: Object,
    #[serde(default)]
    headers: Headers,
    api_key: Option<String>,
    created: u64,
    reply: Option<ReplyStep>,
    chunks: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyStep {
    status: u16,
    body: String,
}

fn failure_value(status: u16) -> Value {
    let mut value = Object::new();
    value.insert(String::from("ok"), Value::Bool(false));
    value.insert(String::from("status"), Value::from(status));
    Value::Object(value)
}

fn reply_value(response: &ChatResponse) -> Value {
    let choice = response.choices.first();
    let mut usage = Object::new();
    usage.insert(
        String::from("prompt_tokens"),
        Value::from(response.usage.prompt_tokens),
    );
    usage.insert(
        String::from("completion_tokens"),
        Value::from(response.usage.completion_tokens),
    );
    usage.insert(
        String::from("total_tokens"),
        Value::from(response.usage.total_tokens),
    );
    let mut value = Object::new();
    value.insert(
        String::from("text"),
        choice
            .and_then(|found| found.message.content.as_deref())
            .map_or(Value::Null, |text| Value::String(text.to_owned())),
    );
    value.insert(
        String::from("finish_reason"),
        choice
            .and_then(|found| found.finish_reason.as_deref())
            .map_or(Value::Null, |text| Value::String(text.to_owned())),
    );
    value.insert(String::from("created"), Value::from(response.created));
    value.insert(String::from("model"), Value::String(response.model.clone()));
    value.insert(String::from("usage"), Value::Object(usage));
    Value::Object(value)
}

fn error_value(error: &NlpCloudError) -> HelperResult {
    let mut value = Object::new();
    value.insert(String::from("status_code"), Value::from(error.status_code));
    value.insert(
        String::from("message"),
        Value::String(error.message.clone()),
    );
    value.insert(
        String::from("headers"),
        serde_json::to_value(&error.headers)?,
    );
    Ok(Value::Object(value))
}

fn reply_from(step: &CompletionStep) -> Result<Value, NlpCloudError> {
    let (status, body) = step.reply.as_ref().map_or((200, DEFAULT_REPLY), |reply| {
        (reply.status, reply.body.as_str())
    });
    let response = decode_reply(ReplyInput {
        status_code: status,
        body: body.as_bytes(),
        model: &step.model,
        created: step.created,
        id: String::from("b8179"),
    })?;
    Ok(reply_value(&response))
}

fn completion_value(step: &CompletionStep) -> HelperResult {
    let Ok(config) = NlpCloudConfig::new(step.defaults.clone()) else {
        return Ok(failure_value(FAILURE_STATUS));
    };
    let mut options = step.optional_params.clone();
    config.merge_defaults(&mut options);
    let body = match transform_request(&step.messages, &options) {
        Ok(body) => body,
        Err(error) => return Ok(failure_value(error.status_code)),
    };
    let reply = if step.chunks.is_some() {
        Value::Null
    } else {
        match reply_from(step) {
            Ok(reply) => reply,
            Err(error) => return Ok(failure_value(error.status_code)),
        }
    };
    let key = step.api_key.as_deref().map(SecretString::from);
    let headers = validate_environment(&step.headers, key.as_ref());
    let mut value = Object::new();
    value.insert(String::from("ok"), Value::Bool(true));
    value.insert(
        String::from("url"),
        Value::String(complete_url(&step.api_base, &step.model)),
    );
    value.insert(String::from("headers"), serde_json::to_value(&headers)?);
    value.insert(String::from("body"), Value::Object(body));
    value.insert(String::from("reply"), reply);
    value.insert(String::from("chunks"), chunk_value(step.chunks.as_deref())?);
    Ok(Value::Object(value))
}

fn chunk_value(chunks: Option<&[String]>) -> HelperResult {
    let Some(lines) = chunks else {
        return Ok(Value::Null);
    };
    let cleaned = clean_and_iterate_chunks(lines.iter().map(String::as_str));
    Ok(serde_json::to_value(cleaned)?)
}

fn supported_value() -> Value {
    Value::Array(
        SUPPORTED_OPENAI_PARAMS
            .iter()
            .map(|name| Value::String((*name).to_owned()))
            .collect(),
    )
}

fn headers_value(headers: &Headers, api_key: Option<&str>) -> HelperResult {
    let key = api_key.map(SecretString::from);
    Ok(serde_json::to_value(validate_environment(
        headers,
        key.as_ref(),
    ))?)
}

fn defaults_value(defaults: &Object) -> Value {
    let Ok(config) = NlpCloudConfig::new(defaults.clone()) else {
        return failure_value(FAILURE_STATUS);
    };
    let mut merged = Object::new();
    config.merge_defaults(&mut merged);
    Value::Object(merged)
}

fn reply_step(status: u16, body: &str, model: &str, created: u64) -> Value {
    let input = ReplyInput {
        status_code: status,
        body: body.as_bytes(),
        model,
        created,
        id: String::from("b8179"),
    };
    match decode_reply(input) {
        Ok(response) => {
            let mut value = Object::new();
            value.insert(String::from("ok"), Value::Bool(true));
            value.insert(String::from("reply"), reply_value(&response));
            Value::Object(value)
        }
        Err(error) => failure_value(error.status_code),
    }
}

fn stream_lines_value(lines: &[String]) -> HelperResult {
    Ok(serde_json::to_value(clean_and_iterate_chunks(
        lines.iter().map(String::as_str),
    ))?)
}

fn execute(step: &Step) -> HelperResult {
    match step {
        Step::SupportedParams => Ok(supported_value()),
        Step::MapParams {
            params,
            optional_params,
        } => Ok(Value::Object(map_openai_params(
            params,
            optional_params.clone(),
        ))),
        Step::Headers { headers, api_key } => headers_value(headers, api_key.as_deref()),
        Step::ErrorClass {
            message,
            status,
            headers,
        } => error_value(&get_error_class(message, *status, headers.clone())),
        Step::Defaults { defaults } => Ok(defaults_value(defaults)),
        Step::Completion(found) => completion_value(found),
        Step::Reply {
            status,
            body,
            model,
            created,
        } => Ok(reply_step(*status, body, model, *created)),
        Step::StreamLines { lines } => stream_lines_value(lines),
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        input
            .steps
            .iter()
            .map(execute)
            .collect::<Result<Vec<Value>, _>>()
            .map(Value::Array)
    }
}
