use std::collections::BTreeMap;
use std::future::ready;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_anthropic::credentials::models::{ModelsTransport, get_models};
use betterlite_anthropic::credentials::{
    ANTHROPIC_VERSION, AnthropicSettings, AuthHeader, OauthTokenClaim, get_auth_header,
    get_base_model, handle_anthropic_oauth, is_anthropic_oauth_key, make_api_key_auth_header,
    resolve_oauth_token_claim,
};
use betterlite_config::sections::anthropic_credentials::Settings as ConfigSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
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
                "api_base": input.api_base.as_deref().unwrap_or(&settings.api_base),
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
            Function::HandleOauth => Ok(handle_oauth(input)),
            Function::ApiKeyHeader => api_key_header(input),
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

fn api_key_header(input: &Input) -> HelperResult {
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

fn handle_oauth(input: &Input) -> Value {
    let mut headers: IndexMap<String, String> = input
        .headers
        .iter()
        .map(|header| (header.name.clone(), header.value.clone()))
        .collect();
    let api_key = input.api_key.clone().map(SecretString::from);
    let resolved = handle_anthropic_oauth(&mut headers, api_key);
    json!({
        "headers": headers,
        "api_key": resolved.as_ref().map(ExposeSecret::expose_secret),
    })
}

struct Fixture<'reply> {
    reply: Option<&'reply Reply>,
    seen: Mutex<Option<(String, Map<String, Value>)>>,
}

impl ModelsTransport for Fixture<'_> {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let headers = request
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    Value::String(String::from_utf8_lossy(value.as_bytes()).into_owned()),
                )
            })
            .collect();
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((request.url.to_string(), headers));
        ready(
            self.reply
                .ok_or(TransportError::Closed)
                .map(|reply| UpstreamResponse {
                    status: reply.status,
                    headers: HeaderMap::new(),
                    body: reply.body.clone().into_bytes(),
                }),
        )
    }
}

fn models(input: &Input, settings: &AnthropicSettings) -> HelperResult {
    let api_key = input.api_key.clone().map(SecretString::from);
    let transport = Fixture {
        reply: input.reply.as_ref(),
        seen: Mutex::new(None),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)?;
    let names = runtime
        .block_on(get_models(
            &transport,
            api_key.as_ref(),
            input.api_base.as_deref(),
            settings,
            Duration::from_secs(30),
        ))
        .map_err(failure)?;
    let seen = transport
        .seen
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    let Some((url, headers)) = seen else {
        return Err(failure("the models fetch sent no request"));
    };
    Ok(json!({
        "url": url,
        "headers": headers,
        "models": names,
    }))
}

fn header_object(header: Option<AuthHeader>) -> Value {
    header.map_or(Value::Null, |found| {
        let mut fields = Map::new();
        fields.insert(
            found.name.to_owned(),
            Value::String(found.value.expose_secret().to_owned()),
        );
        Value::Object(fields)
    })
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
