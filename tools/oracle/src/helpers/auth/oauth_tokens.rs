use std::fmt::Display;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_key_lookup::key_identity;
use betterlite_auth::oauth_tokens::check::{CheckError, check_oauth2_token};
use betterlite_auth::oauth_tokens::transport::{
    TokenInfoRequest, TokenInfoResponse, TokenInfoTransport, TokenInfoTransportError,
};
use betterlite_config::sections::oauth_tokens::{
    DEFAULT_USER_ID_FIELD, DEFAULT_USER_ROLE_FIELD, DEFAULT_USER_TEAM_ID_FIELD, Settings,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::SecretString;
use serde::Serialize;
use serde_json::json;
use tokio::runtime::Builder;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    token: String,
    #[serde(default)]
    endpoint: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    user_id_field_name: Option<String>,
    #[serde(default)]
    user_role_field_name: Option<String>,
    #[serde(default)]
    user_team_id_field_name: Option<String>,
    response: Upstream,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Upstream {
    status: u16,
    body: String,
    #[serde(default)]
    failure: Option<Failure>,
}

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Failure {
    Network,
}

#[derive(Serialize)]
struct RequestView {
    method: String,
    url: String,
    authorization: Option<String>,
    content_type: Option<String>,
    form: Vec<(String, String)>,
}

#[derive(Serialize)]
struct IdentityView {
    user_id: Option<String>,
    user_role: Option<String>,
    team_id: Option<String>,
}

#[derive(Serialize)]
struct ErrorView {
    class: String,
    category: String,
    message: String,
}

struct Outcome {
    request: Option<RequestView>,
    identity: Option<IdentityView>,
    error: Option<ErrorView>,
}

struct Scripted {
    status: u16,
    body: String,
    failure: Option<Failure>,
    recorded: Mutex<Vec<RequestView>>,
}

impl Scripted {
    fn take(&self) -> Option<RequestView> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
    }
}

#[async_trait]
impl TokenInfoTransport for Scripted {
    async fn send(
        &self,
        request: TokenInfoRequest,
    ) -> Result<TokenInfoResponse, TokenInfoTransportError> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(view(&request));
        if matches!(self.failure, Some(Failure::Network)) {
            return Err(TokenInfoTransportError::Failed {
                message: "the identity provider is unreachable".to_owned(),
            });
        }
        Ok(TokenInfoResponse {
            status: self.status,
            body: self.body.clone().into_bytes(),
        })
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(runtime_error)?;
        let outcome = runtime.block_on(check(input));
        Ok(json!({
            "request": outcome.request,
            "identity": outcome.identity,
            "error": outcome.error,
        }))
    }
}

// the production check prepares the request, sends it through the scripted provider and builds the auth
async fn check(input: &Input) -> Outcome {
    let transport = Scripted {
        status: input.response.status,
        body: input.response.body.clone(),
        failure: input.response.failure,
        recorded: Mutex::new(Vec::new()),
    };
    let token = SecretString::from(input.token.clone());
    let configured = settings(input);
    match check_oauth2_token(&configured, &transport, &token).await {
        Ok(auth) => {
            let identity = key_identity(&auth);
            Outcome {
                request: transport.take(),
                identity: Some(IdentityView {
                    user_id: identity.user_id,
                    user_role: identity.user_role.map(|role| role.to_string()),
                    team_id: identity.team_id,
                }),
                error: None,
            }
        }
        Err(error) => Outcome {
            request: transport.take(),
            identity: None,
            error: Some(error_view(&error)),
        },
    }
}

fn settings(input: &Input) -> Settings {
    Settings {
        token_info_endpoint: input.endpoint.clone(),
        user_id_field_name: input
            .user_id_field_name
            .clone()
            .unwrap_or_else(|| DEFAULT_USER_ID_FIELD.to_owned()),
        user_role_field_name: input
            .user_role_field_name
            .clone()
            .unwrap_or_else(|| DEFAULT_USER_ROLE_FIELD.to_owned()),
        user_team_id_field_name: input
            .user_team_id_field_name
            .clone()
            .unwrap_or_else(|| DEFAULT_USER_TEAM_ID_FIELD.to_owned()),
        client_id: input.client_id.clone(),
        client_secret: input.client_secret.clone().map(SecretString::from),
        config_mappings: None,
        enable_oauth2_auth: true,
        enable_oauth2_proxy_auth: false,
    }
}

fn view(request: &TokenInfoRequest) -> RequestView {
    RequestView {
        method: request.method.as_str().to_owned(),
        url: request.url.as_str().to_owned(),
        authorization: header(request, "authorization"),
        content_type: header(request, "content-type"),
        form: request.form.clone(),
    }
}

fn header(request: &TokenInfoRequest, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

// the Python check raises every failure as a ValueError in one of these three message categories
fn error_view(error: &CheckError) -> ErrorView {
    match error {
        CheckError::EndpointAbsent => ErrorView {
            class: "ValueError".to_owned(),
            category: "endpoint_absent".to_owned(),
            message: error.to_string(),
        },
        CheckError::Status { .. } => ErrorView {
            class: "ValueError".to_owned(),
            category: "status".to_owned(),
            message: error.to_string(),
        },
        CheckError::Validation { .. } => ErrorView {
            class: "ValueError".to_owned(),
            category: "validation".to_owned(),
            message: error.to_string(),
        },
    }
}

fn runtime_error<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "RuntimeError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
