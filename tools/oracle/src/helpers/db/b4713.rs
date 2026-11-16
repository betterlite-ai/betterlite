use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::mcp_user_credentials::types::OAuthCredentialPayload;
use betterlite_db::mcp_user_credentials::{is_oauth_credential_expired, oauth_grant_state};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::Utc;
use serde::Deserialize;
use tokio::runtime::Builder;

mod calls;
mod rows;

// the value LiteLLM's MCP_PER_USER_TOKEN_EXPIRY_BUFFER_SECONDS holds when the environment does not set it
const TOKEN_EXPIRY_BUFFER_SECONDS: i64 = 60;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    GetCredential(GetCredential),
    GetOauthCredential(GetOauthCredential),
    StoreCredential(StoreCredential),
    StoreOauthCredential(StoreOauthCredential),
    DeleteCredential(DeleteCredential),
    IsExpired(IsExpired),
    GrantState(GrantState),
    ListServerCredentials(ListServerCredentials),
    ListUserOauthCredentials(ListUserOauthCredentials),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetCredential {
    salt_key: String,
    user_id: String,
    server_id: String,
    rows: Vec<JsonData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetOauthCredential {
    salt_key: String,
    user_id: String,
    server_id: String,
    rows: Vec<JsonData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoreCredential {
    salt_key: String,
    user_id: String,
    server_id: String,
    credential: String,
    rows: Vec<JsonData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoreOauthCredential {
    salt_key: String,
    user_id: String,
    server_id: String,
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    scopes: Option<Vec<String>>,
    skip_byok_guard: bool,
    identity_binding_proof: Option<String>,
    rows: Vec<JsonData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeleteCredential {
    user_id: String,
    server_id: String,
    rows: Vec<JsonData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IsExpired {
    credential: OAuthCredentialPayload,
    buffer_seconds: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GrantState {
    credential: Option<OAuthCredentialPayload>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListServerCredentials {
    salt_key: String,
    server_id: String,
    rows: Vec<JsonData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListUserOauthCredentials {
    salt_key: String,
    user_id: String,
    rows: Vec<JsonData>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        runtime.block_on(async {
            match input {
                Input::GetCredential(call) => calls::get_credential(call).await,
                Input::GetOauthCredential(call) => calls::get_oauth_credential(call).await,
                Input::StoreCredential(call) => calls::store_credential(call).await,
                Input::StoreOauthCredential(call) => calls::store_oauth_credential(call).await,
                Input::DeleteCredential(call) => calls::delete_credential(call).await,
                Input::IsExpired(call) => Ok(serde_json::Value::Bool(is_oauth_credential_expired(
                    &call.credential,
                    Utc::now(),
                    call.buffer_seconds,
                ))),
                Input::GrantState(call) => {
                    let state = oauth_grant_state(
                        call.credential.as_ref(),
                        Utc::now(),
                        TOKEN_EXPIRY_BUFFER_SECONDS,
                    );
                    serde_json::to_value(state).or_value_error()
                }
                Input::ListServerCredentials(call) => calls::list_server_credentials(call).await,
                Input::ListUserOauthCredentials(call) => calls::list_user_oauth(call).await,
            }
        })
    }
}
