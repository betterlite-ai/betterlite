mod tests;

use serde::Deserialize;

use super::ConsentError;
use crate::byok_authorization_html::{AuthorizePage, build_authorize_html};
use crate::mcp_registry_resolve::{resolve, server_initial};
use crate::redirect_trust::loopback::validate_loopback_redirect_uri;
use crate::registry::servers::ServerRegistry;

pub const DEFAULT_CODE_CHALLENGE_METHOD: &str = "S256";

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GetQuery {
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub response_type: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub state: Option<String>,
    pub server_id: Option<String>,
}

impl GetQuery {
    // Starlette's QueryParams keeps the last value of a repeated name.
    pub fn set(&mut self, name: &str, value: String) {
        match name {
            "client_id" => self.client_id = Some(value),
            "redirect_uri" => self.redirect_uri = Some(value),
            "response_type" => self.response_type = Some(value),
            "code_challenge" => self.code_challenge = Some(value),
            "code_challenge_method" => self.code_challenge_method = Some(value),
            "state" => self.state = Some(value),
            "server_id" => self.server_id = Some(value),
            _unknown => {}
        }
    }
}

// LiteLLM's byok_authorize_get: validate the OAuth request, then render the consent form.
pub fn authorize_get(
    query: &GetQuery,
    registry: Option<&ServerRegistry>,
) -> Result<String, ConsentError> {
    if query.response_type.as_deref() != Some("code") {
        return Err(ConsentError::ResponseType);
    }
    let Some(redirect_uri) = query.redirect_uri.as_deref().filter(|uri| !uri.is_empty()) else {
        return Err(ConsentError::RedirectUriRequired);
    };
    validate_loopback_redirect_uri(redirect_uri)?;
    let Some(code_challenge) = query
        .code_challenge
        .as_deref()
        .filter(|challenge| !challenge.is_empty())
    else {
        return Err(ConsentError::CodeChallengeRequired);
    };
    let metadata = resolve(query.server_id.as_deref(), registry);
    let page = AuthorizePage {
        server_initial: server_initial(&metadata.server_name),
        server_name: metadata.server_name,
        client_id: query.client_id.clone().unwrap_or_default(),
        redirect_uri: redirect_uri.to_owned(),
        code_challenge: code_challenge.to_owned(),
        code_challenge_method: query
            .code_challenge_method
            .clone()
            .filter(|method| !method.is_empty())
            .unwrap_or_else(|| DEFAULT_CODE_CHALLENGE_METHOD.to_owned()),
        state: query.state.clone().unwrap_or_default(),
        server_id: query.server_id.clone().unwrap_or_default(),
        access_items: metadata.access_items,
        help_url: metadata.help_url,
    };
    Ok(build_authorize_html(&page))
}
