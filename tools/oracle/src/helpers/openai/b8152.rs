use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::local_servers::oobabooga::error::{OobaboogaError, ReplyError};
use betterlite_openai::local_servers::oobabooga::reply::{
    transform_embedding_response, transform_response,
};
use betterlite_openai::local_servers::oobabooga::{
    completion_body, completion_url, embedding_body, embeddings_url, validate_environment,
};
use betterlite_openai::local_servers::{
    ProviderEnvironment, docker_model_runner, lemonade_cost, llamafile, lm_studio, xinference_image,
};
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};
use betterlite_prompts::arguments::json::Object as JsonObject;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ProviderInfo { calls: Vec<ProviderCall> },
    ChatParams { calls: Vec<ParamsCall> },
    DockerRequest { calls: Vec<DockerCall> },
    LmStudioEmbed { optional_params: Object },
    Xinference { calls: Vec<XinferenceCall> },
    LemonadeCost { calls: Vec<CostCall> },
    OobaboogaRequest { calls: Vec<RequestCall> },
    OobaboogaEmbedding { calls: Vec<EmbeddingCall> },
    OobaboogaReply { calls: Vec<ReplyCall> },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ServerProvider {
    DockerModelRunner,
    Llamafile,
    LmStudio,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChatProvider {
    DockerModelRunner,
    LmStudio,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderCall {
    provider: ServerProvider,
    api_base: Option<String>,
    api_key: Option<String>,
    env: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParamsCall {
    provider: ChatProvider,
    model: String,
    params: Object,
    preset: Object,
    chat_models: BTreeSet<String>,
    text_models: BTreeSet<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DockerCall {
    api_base: Option<String>,
    model: String,
    messages: Vec<ChatMessage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct XinferenceCall {
    model: String,
    params: Object,
    preset: Object,
    drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CostCall {
    model: String,
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestCall {
    model: String,
    api_base: Option<String>,
    api_key: Option<String>,
    messages: Vec<ChatMessage>,
    optional_params: JsonObject,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmbeddingCall {
    model: String,
    api_base: Option<String>,
    api_key: Option<String>,
    input: Vec<String>,
    optional_params: JsonObject,
    reply: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCall {
    model: String,
    status: u16,
    body: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ProviderInfo { calls } => {
                Ok(Value::Array(calls.iter().map(provider_value).collect()))
            }
            Input::ChatParams { calls } => {
                Ok(Value::Array(calls.iter().map(params_value).collect()))
            }
            Input::DockerRequest { calls } => Ok(Value::Array(
                calls
                    .iter()
                    .map(docker_value)
                    .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            )),
            Input::LmStudioEmbed { optional_params } => Ok(embed_value(optional_params)),
            Input::Xinference { calls } => {
                Ok(Value::Array(calls.iter().map(xinference_value).collect()))
            }
            Input::LemonadeCost { calls } => {
                Ok(Value::Array(calls.iter().map(cost_value).collect()))
            }
            Input::OobaboogaRequest { calls } => Ok(Value::Array(
                calls
                    .iter()
                    .map(request_value)
                    .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            )),
            Input::OobaboogaEmbedding { calls } => Ok(Value::Array(
                calls
                    .iter()
                    .map(embedding_value)
                    .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            )),
            Input::OobaboogaReply { calls } => Ok(Value::Array(
                calls
                    .iter()
                    .map(reply_value)
                    .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            )),
        }
    }
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn failure(error: &OobaboogaError) -> Value {
    json!({
        "ok": false,
        "status": error.status_code,
        "message": error.message,
    })
}

fn provider_value(call: &ProviderCall) -> Value {
    let (base_name, key_name) = match call.provider {
        ServerProvider::DockerModelRunner => (
            "DOCKER_MODEL_RUNNER_API_BASE",
            "DOCKER_MODEL_RUNNER_API_KEY",
        ),
        ServerProvider::Llamafile => ("LLAMAFILE_API_BASE", "LLAMAFILE_API_KEY"),
        ServerProvider::LmStudio => ("LM_STUDIO_API_BASE", "LM_STUDIO_API_KEY"),
    };
    let environment = ProviderEnvironment {
        api_base: call.env.get(base_name).cloned(),
        api_key: call.env.get(key_name).cloned().map(SecretString::from),
    };
    let key = call.api_key.clone().map(SecretString::from);
    let base = call.api_base.as_deref();
    let (api_base, api_key) = match call.provider {
        ServerProvider::DockerModelRunner => {
            let info = docker_model_runner::provider_info(base, key.as_ref(), &environment);
            (Some(info.api_base), info.api_key)
        }
        ServerProvider::Llamafile => {
            let info = llamafile::provider_info(base, key.as_ref(), &environment);
            (Some(info.api_base), info.api_key)
        }
        ServerProvider::LmStudio => {
            let info = lm_studio::provider_info(base, key.as_ref(), &environment);
            (info.api_base, info.api_key)
        }
    };
    json!({"api_base": api_base, "api_key": api_key.expose_secret()})
}

fn params_value(call: &ParamsCall) -> Value {
    let catalog = CatalogCapabilities {
        chat_models: call.chat_models.clone(),
        text_models: call.text_models.clone(),
    };
    let base = OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    };
    let mut optional = call.preset.clone();
    match call.provider {
        ChatProvider::DockerModelRunner => {
            let requested: IndexMap<String, Value> = call
                .params
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect();
            docker_model_runner::ChatConfig { base }.map_openai_params(
                &call.model,
                &requested,
                &mut optional,
                &catalog,
            );
        }
        ChatProvider::LmStudio => lm_studio::ChatConfig { base }.map_openai_params(
            &call.model,
            &call.params,
            &mut optional,
            &catalog,
        ),
    }
    Value::Object(optional)
}

fn docker_value(call: &DockerCall) -> Result<Value, Box<HelperError>> {
    let url = docker_model_runner::complete_url(call.api_base.as_deref());
    let Ok(messages) =
        docker_model_runner::transform_messages(&call.model, &call.messages, &RefuseDownloads)
    else {
        return Ok(json!({"url": url, "ok": false}));
    };
    let wire = serde_json::to_value(&messages).map_err(|error| invalid(error.to_string()))?;
    Ok(json!({"url": url, "ok": true, "messages": wire}))
}

fn embed_value(optional_params: &Object) -> Value {
    let config: Object = lm_studio::EMBEDDING_CONFIG_KEYS
        .iter()
        .map(|key| ((*key).to_owned(), Value::Null))
        .collect();
    json!({
        "supported": lm_studio::EMBEDDING_SUPPORTED_PARAMS,
        "config": config,
        "mapped": optional_params,
    })
}

fn xinference_value(call: &XinferenceCall) -> Value {
    let mut optional = call.preset.clone();
    let mapped = xinference_image::map_openai_params(
        &call.model,
        &call.params,
        &mut optional,
        call.drop_params,
    );
    match mapped {
        Ok(()) => json!({
            "ok": true,
            "supported": xinference_image::SUPPORTED_PARAMS,
            "value": optional,
        }),
        Err(_) => json!({"ok": false, "supported": xinference_image::SUPPORTED_PARAMS}),
    }
}

fn cost_value(call: &CostCall) -> Value {
    let (prompt_rate, completion_rate) = lemonade_cost::COST_PER_TOKEN;
    let prompt = prompt_rate.checked_mul(Decimal::from(call.prompt_tokens));
    let completion = completion_rate.checked_mul(Decimal::from(call.completion_tokens));
    json!({
        "model": call.model,
        "prompt_is_zero": prompt.is_some_and(|cost| cost.is_zero()),
        "completion_is_zero": completion.is_some_and(|cost| cost.is_zero()),
    })
}

fn wire_headers(api_key: Option<&str>) -> Headers {
    let key = api_key.map(|text| SecretString::from(text.to_owned()));
    validate_environment(key.as_ref())
        .into_iter()
        .map(|(name, value)| (name.to_lowercase(), value))
        .collect()
}

fn request_value(call: &RequestCall) -> Result<Value, Box<HelperError>> {
    let url = match completion_url(&call.model, call.api_base.as_deref()) {
        Ok(url) => url,
        Err(error) => return Ok(failure(&error)),
    };
    let body = completion_body(
        &call.model,
        &call.messages,
        &call.optional_params,
        &RefuseDownloads,
    )
    .map_err(|error| invalid(error.to_string()))?;
    let headers = wire_headers(call.api_key.as_deref());
    Ok(json!({"ok": true, "request": {"url": url, "headers": headers, "body": body}}))
}

fn embedding_value(call: &EmbeddingCall) -> Result<Value, Box<HelperError>> {
    let url = match embeddings_url(&call.model, call.api_base.as_deref()) {
        Ok(url) => url,
        Err(error) => return Ok(failure(&error)),
    };
    let input = EmbeddingInput::Texts(call.input.clone());
    let body = embedding_body(&input, &call.optional_params)
        .map_err(|error| invalid(error.to_string()))?;
    let headers = wire_headers(call.api_key.as_deref());
    match transform_embedding_response(&call.model, &call.reply) {
        Ok(reply) => Ok(json!({
            "ok": true,
            "request": {"url": url, "headers": headers, "body": body},
            "model": reply.model,
            "data": [{"embedding": reply.embedding, "index": 0, "object": "embedding"}],
            "prompt_tokens": reply.prompt_tokens,
            "total_tokens": reply.total_tokens,
        })),
        Err(ReplyError::Server(error)) => Ok(failure(&error)),
        Err(other) => Err(invalid(other.to_string())),
    }
}

// the creation time is the wall clock, so the comparison leaves it out
fn reply_value(call: &ReplyCall) -> Result<Value, Box<HelperError>> {
    match transform_response(&call.model, call.status, &call.body, 0) {
        Ok(reply) => Ok(json!({
            "ok": true,
            "model": reply.model,
            "content": reply.content,
            "usage": reply.usage,
        })),
        Err(ReplyError::Server(error)) => Ok(failure(&error)),
        Err(other) => Err(invalid(other.to_string())),
    }
}
