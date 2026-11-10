use std::collections::BTreeMap;

use betterlite_anthropic::credentials::{
    ANTHROPIC_VERSION, AnthropicSettings, AuthHeader, OauthTokenClaim, decode_anthropic_models,
    get_api_base, get_auth_header, get_base_model, handle_anthropic_oauth, is_anthropic_oauth_key,
    make_api_key_auth_header, models_request, resolve_oauth_token_claim,
};
use betterlite_config::sections::anthropic_credentials::Settings as ConfigSettings;
use indexmap::IndexMap;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    AuthHeader,
    ApiBase,
    OauthKey,
    UsedOauthToken,
    HandleOauth,
    ApiKeyHeader,
    BaseModel,
    Models,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    status: u16,
    body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    use_bearer_for_custom_base: bool,
    #[serde(default)]
    headers: Vec<Header>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    client_sent_oauth_token: OauthTokenClaim,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    reply: Option<Reply>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let settings = settings(input)?;
        match input.function {
            Function::AuthHeader => {
                let api_key = input.api_key.clone().map(SecretString::from);
                let header = get_auth_header(
                    api_key.as_ref(),
                    input.api_base.as_deref(),
                    input.use_bearer_for_custom_base,
                    &settings,
                );
                Ok(json!({ "header": header_object(header) }))
            }
            Function::ApiBase => Ok(json!({
                "api_base": get_api_base(input.api_base.as_deref(), &settings),
            })),
            Function::OauthKey => Ok(json!({
                "oauth": is_anthropic_oauth_key(input.value.as_deref()),
            })),
            Function::UsedOauthToken => Ok(json!({
                "used": resolve_oauth_token_claim(
                    input.client_sent_oauth_token,
                    input.provider.as_deref(),
                ),
            })),
            Function::HandleOauth => handle_oauth(input),
            Function::ApiKeyHeader => {
                let Some(api_key) = input.api_key.clone().map(SecretString::from) else {
                    return Err(failure("the api_key_header case needs an api_key"));
                };
                let header = make_api_key_auth_header(
                    &api_key,
                    input.api_base.as_deref(),
                    input.use_bearer_for_custom_base,
                );
                Ok(json!({ "header": header_object(Some(header)) }))
            }
            Function::BaseModel => Ok(json!({
                "model": get_base_model(input.model.as_deref()),
            })),
            Function::Models => models(input, &settings),
        }
    }
}

fn settings(input: &Input) -> Result<AnthropicSettings, Box<HelperError>> {
    let environment = ConfigSettings::resolve_environment(&input.environment).map_err(failure)?;
    Ok(AnthropicSettings {
        api_key: environment.api_key,
        auth_token: environment.auth_token,
        api_base: environment.api_base,
        use_bearer_for_custom_base: input.use_bearer_for_custom_base,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    })
}

fn handle_oauth(input: &Input) -> HelperResult {
    let mut headers: IndexMap<String, String> = input
        .headers
        .iter()
        .map(|header| (header.name.clone(), header.value.clone()))
        .collect();
    let api_key = input.api_key.clone().map(SecretString::from);
    let resolved = handle_anthropic_oauth(&mut headers, api_key);
    Ok(json!({
        "headers": headers,
        "api_key": resolved.as_ref().map(ExposeSecret::expose_secret),
    }))
}

fn models(input: &Input, settings: &AnthropicSettings) -> HelperResult {
    let api_key = input.api_key.clone().map(SecretString::from);
    let request =
        models_request(api_key.as_ref(), input.api_base.as_deref(), settings).map_err(failure)?;
    let Some(reply) = &input.reply else {
        return Err(failure("the models case needs a fixture reply"));
    };
    let models = decode_anthropic_models(reply.status, reply.body.as_bytes()).map_err(failure)?;
    Ok(json!({
        "url": request.url,
        "headers": request.headers,
        "models": models,
    }))
}

fn header_object(header: Option<AuthHeader>) -> Value {
    let Some(header) = header else {
        return Value::Null;
    };
    let mut fields = Map::new();
    fields.insert(
        header.name.to_owned(),
        Value::String(header.value.expose_secret().to_owned()),
    );
    Value::Object(fields)
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: std::fmt::Display,
{
    Box::new(HelperError {
        class: "Error".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
