use url::Url;

use crate::oauth_client::OAuthClient;
use crate::oauth_client::error::{OAuthClientError, OAuthEndpoint, endpoint_url, require_secure};

#[derive(Debug, Clone)]
pub struct AuthorizationRequest {
    pub endpoint: String,
    pub state: Option<String>,
    pub redirect_uri: Option<String>,
    pub extra_params: Vec<(String, String)>,
}

pub fn authorization_url(
    client: &OAuthClient,
    request: &AuthorizationRequest,
) -> Result<Url, OAuthClientError> {
    let mut url = endpoint_url(&request.endpoint, OAuthEndpoint::Authorization)?;
    require_secure(client, &url, OAuthEndpoint::Authorization)?;
    let redirect_uri = request
        .redirect_uri
        .as_deref()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            client
                .redirect_uri
                .as_deref()
                .filter(|value| !value.is_empty())
        });
    let Some(configured) = redirect_uri else {
        return Err(OAuthClientError::MissingRedirectUri);
    };
    let mut params: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    params.push(("response_type".to_owned(), "code".to_owned()));
    params.push(("client_id".to_owned(), client.client_id.clone()));
    params.push(("redirect_uri".to_owned(), configured.to_owned()));
    if !client.scopes.is_empty() {
        params.push(("scope".to_owned(), client.scopes.join(" ")));
    }
    if let Some(state) = request.state.as_deref().filter(|value| !value.is_empty()) {
        params.push(("state".to_owned(), state.to_owned()));
    }
    for (name, value) in &request.extra_params {
        if !value.is_empty() {
            params.push((name.clone(), value.clone()));
        }
    }
    url.query_pairs_mut().clear().extend_pairs(params);
    Ok(url)
}
