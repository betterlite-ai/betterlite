use betterlite_auth::oauth_client::document::OAuthUserInfo;
use betterlite_auth::oauth_client::google::{GoogleIdentityError, google_open_id};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    userinfo: OAuthUserInfo,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let identity = google_open_id(&input.userinfo).map_err(|error| login_error(&error))?;
        Ok(serde_json::to_value(identity)?)
    }
}

// fastapi-sso raises its SSOLoginError for an unverified Google account, and the oracle compares that status
fn login_error(error: &GoogleIdentityError) -> Box<HelperError> {
    let response = error.response();
    Box::new(HelperError {
        class: "SSOLoginError".to_owned(),
        status: Some(response.status),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}
