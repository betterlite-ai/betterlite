use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_http::gigachat_tokens::authenticator::GigachatAuthenticator;
use betterlite_http::gigachat_tokens::reply::decode_token_reply;
use betterlite_http::gigachat_tokens::request::GigachatTokenRequest;
use betterlite_http::gigachat_tokens::runtime::SystemRuntime;
use betterlite_http::gigachat_tokens::tokens::GigachatTokens as _;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    access_token: Option<String>,
    credentials: Option<String>,
    scope: Option<String>,
    auth_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    AccessToken {
        #[serde(default)]
        request: Request,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    Reply {
        body: String,
    },
}

fn failure(class: &str, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn token_request(request: &Request) -> GigachatTokenRequest {
    GigachatTokenRequest {
        access_token: request.access_token.clone().map(SecretString::from),
        credentials: request.credentials.clone().map(SecretString::from),
        scope: request.scope.clone(),
        auth_url: request.auth_url.clone(),
    }
}

fn access_token(request: &Request, environment: &BTreeMap<String, String>) -> HelperResult {
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load("")
    .map_err(|error| failure("ValueError", None, error.to_string()))?;
    let client = reqwest::ClientBuilder::new()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| failure("OSError", None, error.to_string()))?;
    let auth = GigachatAuthenticator::new(config.sections.gigachat_auth, client, SystemRuntime);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("RuntimeError", None, error.to_string()))?;
    match runtime.block_on(auth.access_token(&token_request(request))) {
        Ok(token) => Ok(json!({"token": token.expose_secret()})),
        Err(error) => Err(failure(
            "GigaChatAuthError",
            Some(error.status_code().as_u16()),
            error.to_string(),
        )),
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::AccessToken {
                request,
                environment,
            } => access_token(request, environment),
            Input::Reply { body } => match decode_token_reply(body.as_bytes()) {
                Ok(reply) => Ok(json!({
                    "token": reply.token.expose_secret(),
                    "expires_at": reply.expires_at,
                })),
                Err(error) => Err(failure(
                    "GigaChatAuthError",
                    Some(error.status_code().as_u16()),
                    error.to_string(),
                )),
            },
        }
    }
}
