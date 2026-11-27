use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::subject_state::{active_key_user_id, key_is_active, user_scim_deactivated};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::mcp::active_subjects::presented_key::litellm_key_from_request;
use chrono::Utc;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::json;

mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    PresentedKey { headers: Vec<(String, String)> },
    KeyIsActive { key: KeyFields },
    ActiveKeyUserId { key: KeyFields },
    UserRecord { user: LiteLLMUserTable },
}

// the case's key object, built into a UserAPIKeyAuth the way LiteLLM's pydantic model takes it
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyFields {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    blocked: Option<bool>,
    #[serde(default)]
    expires: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input {
            Input::PresentedKey { headers } => {
                let mut map = HeaderMap::new();
                for name in headers {
                    add_header(&mut map, name)?;
                }
                Ok(json!(litellm_key_from_request(&map)))
            }
            Input::KeyIsActive { key } => {
                let auth = auth_from(key)?;
                Ok(json!(key_is_active(&auth, Utc::now())))
            }
            Input::ActiveKeyUserId { key } => {
                let auth = auth_from(key)?;
                Ok(json!(active_key_user_id(&auth, Utc::now())))
            }
            Input::UserRecord { user } => {
                let answer = if user_scim_deactivated(user) {
                    "no_active_key"
                } else {
                    "active"
                };
                Ok(json!(answer))
            }
        }
    }
}

fn add_header(map: &mut HeaderMap, header: &(String, String)) -> Result<(), Box<HelperError>> {
    let name = HeaderName::from_bytes(header.0.as_bytes()).or_value_error()?;
    let value = HeaderValue::from_bytes(&latin1(&header.1).or_value_error()?).or_value_error()?;
    map.append(name, value);
    Ok(())
}

// Starlette decodes a raw header byte as the character of the same code point
fn latin1(text: &str) -> Result<Vec<u8>, String> {
    text.chars()
        .map(|character| {
            u8::try_from(u32::from(character))
                .map_err(|_error| format!("the header text {text:?} is not Latin-1"))
        })
        .collect()
}

fn auth_from(key: &KeyFields) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let mut fields = BTreeMap::new();
    if let Some(token) = &key.token {
        fields.insert("token".to_owned(), JsonData::String(token.clone()));
    }
    if let Some(user_id) = &key.user_id {
        fields.insert("user_id".to_owned(), JsonData::String(user_id.clone()));
    }
    if let Some(team_id) = &key.team_id {
        fields.insert("team_id".to_owned(), JsonData::String(team_id.clone()));
    }
    if let Some(blocked) = key.blocked {
        fields.insert("blocked".to_owned(), JsonData::Bool(blocked));
    }
    if let Some(expires) = &key.expires {
        fields.insert("expires".to_owned(), JsonData::String(expires.clone()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()
}
