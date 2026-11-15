use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use tracing::debug;

use crate::databricks_auth::Headers;
use crate::databricks_auth::endpoint::EndpointType;
use crate::databricks_auth::environment::DatabricksEnv;
use crate::databricks_auth::error::{
    DatabricksError, MISSING_API_BASE, MISSING_API_KEY, SDK_CREDENTIALS,
};
use crate::databricks_auth::oauth::{OauthTokenSource, oauth_token_request, request_oauth_token};
use crate::databricks_auth::redaction::redact_headers_for_logging;
use crate::databricks_auth::user_agent::build_user_agent;

pub struct ValidationRequest<'value> {
    pub api_key: Option<&'value SecretString>,
    pub api_base: Option<&'value str>,
    pub endpoint: EndpointType,
    pub custom_endpoint: bool,
    pub headers: Option<Headers>,
    pub custom_user_agent: Option<&'value str>,
}

struct AuthenticationRequest<'value> {
    api_key: Option<&'value SecretString>,
    api_base: Option<String>,
    custom_endpoint: bool,
    headers: Option<Headers>,
}

fn oauth_credentials<'value>(
    env: &'value DatabricksEnv,
    api_base: Option<&'value str>,
) -> Option<(&'value str, &'value SecretString, &'value str)> {
    let client_id = env.client_id.as_deref().filter(|value| !value.is_empty())?;
    let client_secret = env
        .client_secret
        .as_ref()
        .filter(|value| !value.expose_secret().is_empty())?;
    let base = api_base.filter(|value| !value.is_empty())?;
    Some((client_id, client_secret, base))
}

async fn authenticate(
    request: AuthenticationRequest<'_>,
    env: &DatabricksEnv,
    tokens: &dyn OauthTokenSource,
) -> Result<(String, Headers), DatabricksError> {
    let AuthenticationRequest {
        api_key,
        api_base,
        custom_endpoint,
        headers: mut headers,
    } = request;
    let oauth = oauth_credentials(env, api_base.as_deref());
    if let Some((client_id, client_secret, base)) = oauth {
        debug!("Using OAuth M2M authentication for Databricks");
        let token_request = oauth_token_request(base, client_id, client_secret.expose_secret());
        let token = request_oauth_token(&token_request, tokens).await?;
        headers = Some(oauth_headers(headers, token.expose_secret()));
    }
    let base = required_base(
        api_key,
        api_base.as_deref(),
        custom_endpoint,
        headers.as_ref(),
    )?;
    Ok((base, with_api_key(api_key, headers)))
}

// LiteLLM's missing-key 400 sits in the OAuth branch's else arm, since OAuth sets a header map and a base; the SDK text follows when no custom endpoint is set
fn required_base(
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
    custom_endpoint: bool,
    headers: Option<&Headers>,
) -> Result<String, DatabricksError> {
    if api_key.is_none() && headers.is_none_or(BTreeMap::is_empty) {
        return Err(DatabricksError {
            status_code: 400,
            message: if custom_endpoint {
                MISSING_API_KEY
            } else {
                SDK_CREDENTIALS
            }
            .to_owned(),
            headers: None,
        });
    }
    let Some(base) = api_base else {
        let message = if custom_endpoint {
            MISSING_API_BASE
        } else {
            SDK_CREDENTIALS
        };
        return Err(DatabricksError {
            status_code: 400,
            message: message.to_owned(),
            headers: None,
        });
    };
    Ok(base.to_owned())
}

// LiteLLM's OAuth branch adds the bearer token and the JSON content type to the request headers
fn oauth_headers(headers: Option<Headers>, token: &str) -> Headers {
    let mut resolved = headers.unwrap_or_default();
    resolved.extend([
        ("Authorization".to_owned(), format!("Bearer {token}")),
        ("Content-Type".to_owned(), "application/json".to_owned()),
    ]);
    resolved
}

fn with_api_key(api_key: Option<&SecretString>, headers: Option<Headers>) -> Headers {
    let mut resolved = headers.unwrap_or_else(|| {
        // a missing key is spelled out because LiteLLM writes the f-string over a None
        BTreeMap::from([
            ("Authorization".to_owned(), "Bearer None".to_owned()),
            ("Content-Type".to_owned(), "application/json".to_owned()),
        ])
    });
    if let Some(key) = api_key {
        resolved.insert(
            "Authorization".to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    resolved
}

pub async fn validate_environment(
    request: ValidationRequest<'_>,
    env: &DatabricksEnv,
    tokens: &dyn OauthTokenSource,
) -> Result<(String, Headers), DatabricksError> {
    let custom_user_agent = request.custom_user_agent;
    let endpoint = request.endpoint;
    let custom_endpoint = request.custom_endpoint;
    let api_base = request
        .api_base
        .map(str::to_owned)
        .or_else(|| env.api_base.clone());
    let (base, mut headers) = authenticate(
        AuthenticationRequest {
            api_key: request.api_key,
            api_base,
            custom_endpoint,
            headers: request.headers,
        },
        env,
        tokens,
    )
    .await?;
    headers.insert(
        "User-Agent".to_owned(),
        build_user_agent(custom_user_agent, &env.version),
    );
    debug!(
        headers = ?redact_headers_for_logging(&headers),
        "Databricks request headers"
    );
    let url = match (endpoint, custom_endpoint) {
        (_, true) => base,
        (EndpointType::ChatCompletions, false) => format!("{base}/chat/completions"),
        (EndpointType::Embeddings, false) => format!("{base}/embeddings"),
    };
    Ok((url, headers))
}
