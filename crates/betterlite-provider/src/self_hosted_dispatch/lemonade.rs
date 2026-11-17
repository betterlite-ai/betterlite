use std::time::Duration;

use betterlite_models::decisions::Json as DecisionsJson;
use betterlite_models::lemonade_model_metadata::{
    DeploymentCredentials, LemonadeCredentials, ModelInfoClient, ModelInfoRequest, auth_header,
    resolve_origin,
};
use betterlite_models::provider_metadata_factory::adapters::LemonadeSettings;
use betterlite_openai::compatible_wire::{
    CompatibleReply, ReplyOptions, decode_compatible_chat_reply,
};
use betterlite_prompts::arguments::json::{Json, Object};

use super::error::{DispatchError, LemonadeError};
use super::logging;

pub const PROVIDER: &str = "lemonade";
const MODELS_PATH: &str = "/models";
const DATA: &str = "data";
const ID: &str = "id";
const REPEAT_PENALTY: &str = "repeat_penalty";
const FUNCTIONS: &str = "functions";
const LOGIT_BIAS: &str = "logit_bias";
const MAX_TOKENS: &str = "max_tokens";
const MAX_COMPLETION_TOKENS: &str = "max_completion_tokens";
const NUMBER: &str = "n";
const PRESENCE_PENALTY: &str = "presence_penalty";
const STOP: &str = "stop";
const TEMPERATURE: &str = "temperature";
const TOP_P: &str = "top_p";
const TOP_K: &str = "top_k";
const RESPONSE_FORMAT: &str = "response_format";
const TOOLS: &str = "tools";

// LemonadeChatConfig.get_config: the class body's fields that the constructor set, in that order
pub fn get_config(settings: &LemonadeSettings) -> Result<Object, DispatchError> {
    let fields = [
        (REPEAT_PENALTY, logging::convert(&settings.repeat_penalty)?),
        (FUNCTIONS, logging::convert(&settings.functions)?),
        (LOGIT_BIAS, logging::convert(&settings.logit_bias)?),
        (MAX_TOKENS, logging::convert(&settings.max_tokens)?),
        (
            MAX_COMPLETION_TOKENS,
            logging::convert(&settings.max_completion_tokens)?,
        ),
        (NUMBER, logging::convert(&settings.number)?),
        (
            PRESENCE_PENALTY,
            logging::convert(&settings.presence_penalty)?,
        ),
        (STOP, logging::convert(&settings.stop)?),
        (TEMPERATURE, logging::convert(&settings.temperature)?),
        (TOP_P, logging::convert(&settings.top_p)?),
        (TOP_K, logging::convert(&settings.top_k)?),
        (
            RESPONSE_FORMAT,
            logging::convert(&settings.response_format)?,
        ),
        (TOOLS, logging::convert(&settings.tools)?),
    ];
    let mut config = Object::new();
    for (name, value) in fields {
        if value != Json::Null {
            config.insert(String::from(name), value);
        }
    }
    Ok(config)
}

// LemonadeChatConfig.get_models: GET {base}/models and prefix every id with lemonade/
pub async fn get_models<Client>(
    client: &Client,
    credentials: &LemonadeCredentials,
    deployment: DeploymentCredentials<'_>,
    timeout: Duration,
) -> Result<Vec<String>, LemonadeError>
where
    Client: ModelInfoClient,
{
    let origin = resolve_origin(credentials, deployment);
    let request = ModelInfoRequest {
        url: format!("{}{MODELS_PATH}", origin.api_base),
        headers: auth_header(&origin.api_key).into_iter().collect(),
        timeout,
    };
    let response = client
        .get(&request)
        .await
        .map_err(|error| LemonadeError::Request {
            message: error.to_string(),
        })?;
    if response.status != 200 {
        let status = response.status;
        let body = String::from_utf8(response.body)
            .map_err(|source| LemonadeError::StatusBody { status, source })?;
        return Err(LemonadeError::Status { status, body });
    }
    decode_model_names(&response.body)
}

// LemonadeChatConfig.transform_response: the shared openai_like reply, then the model named lemonade/ plus the request's
pub fn transform_response(
    body: &[u8],
    model: &str,
    options: &ReplyOptions<'_>,
) -> Result<CompatibleReply, LemonadeError> {
    let mut reply = decode_compatible_chat_reply(body, options)
        .map_err(|source| LemonadeError::ChatReply { source })?;
    reply.response.model = format!("{PROVIDER}/{model}");
    Ok(reply)
}

// LemonadeChatConfig.get_models's reply: the data list's string ids, each prefixed with lemonade/
pub fn decode_model_names(body: &[u8]) -> Result<Vec<String>, LemonadeError> {
    let reply: DecisionsJson =
        serde_json::from_slice(body).map_err(|source| LemonadeError::Reply { source })?;
    let DecisionsJson::Object(fields) = &reply else {
        return Err(LemonadeError::Shape);
    };
    let entries = match fields.get(DATA) {
        None => return Ok(Vec::new()),
        Some(DecisionsJson::List(entries)) => entries,
        Some(_) => return Err(LemonadeError::Shape),
    };
    let mut names = Vec::with_capacity(entries.len());
    for entry in entries {
        let DecisionsJson::Object(object) = entry else {
            return Err(LemonadeError::Entry);
        };
        let Some(DecisionsJson::Text(id)) = object.get(ID) else {
            return Err(LemonadeError::Entry);
        };
        names.push(format!("{PROVIDER}/{id}"));
    }
    Ok(names)
}
