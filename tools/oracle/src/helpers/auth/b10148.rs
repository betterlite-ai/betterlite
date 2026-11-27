use std::collections::BTreeMap;

use betterlite_auth::oauth_client::document::OAuthDiscoveryDocument;
use betterlite_auth::oauth_client::microsoft::{MicrosoftClientError, microsoft_sso_client};
use betterlite_config::sections::microsoft_sso::Settings;
use betterlite_oracle_helper_api::error::{ErrorCode, HelperError};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::{Deserialize, Serialize};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    client_id: String,
    redirect_url: String,
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
struct ClientView {
    tenant: String,
    discovery: DiscoveryView,
}

#[derive(Debug, Serialize)]
struct DiscoveryView {
    authorization_endpoint: String,
    token_endpoint: String,
    userinfo_endpoint: String,
}

impl From<&OAuthDiscoveryDocument> for DiscoveryView {
    fn from(document: &OAuthDiscoveryDocument) -> Self {
        Self {
            authorization_endpoint: document.authorization_endpoint.clone(),
            token_endpoint: document.token_endpoint.clone(),
            userinfo_endpoint: document.userinfo_endpoint.clone(),
        }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let settings = Settings::resolve_environment(&input.environment);
        let client = microsoft_sso_client(&settings, &input.client_id, &input.redirect_url)
            .map_err(|error| client_error(&error))?;
        Ok(serde_json::to_value(ClientView {
            tenant: client.tenant,
            discovery: DiscoveryView::from(&client.discovery),
        })?)
    }
}

// the Python side reads the secret from the environment before it builds the client, so no case compares this shape
fn client_error(error: &MicrosoftClientError) -> Box<HelperError> {
    let response = error.response();
    Box::new(HelperError {
        class: "MicrosoftClientError".to_owned(),
        status: Some(response.status),
        error_type: Some(None),
        param: Some(response.param.map(str::to_owned)),
        code: Some(Some(ErrorCode::Text(response.code.to_owned()))),
        message: error.to_string(),
    })
}
