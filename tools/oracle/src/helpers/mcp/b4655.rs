mod tests;

use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_auth::auth_key_lookup::error::KeyLookupError;
use betterlite_auth::auth_key_lookup::{KeyAuthenticator, KeyLookupOutcome};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_mcp::session_admission::auth::byok_session_auth;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::DateTime;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;

#[derive(Deserialize)]
pub(crate) struct Input {
    cookie_header: Option<String>,
    master_key: String,
    key_resolves: bool,
    now: i64,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let Some(now) = DateTime::from_timestamp(input.now, 0) else {
            return Err(Box::new(HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: format!(
                    "the fixed clock is outside the supported range: {}",
                    input.now
                ),
            }));
        };
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let keys = Keys {
            resolves: input.key_resolves,
        };
        let master = SecretString::from(input.master_key.as_str());
        let admitted = runtime
            .block_on(byok_session_auth(
                input.cookie_header.as_deref(),
                &master,
                Some(&keys),
                now,
            ))
            .is_ok();
        Ok(json!({ "admitted": admitted }))
    }
}

struct Keys {
    resolves: bool,
}

#[async_trait]
impl KeyAuthenticator for Keys {
    async fn authenticate(&self, presented: &str) -> Result<KeyLookupOutcome, KeyLookupError> {
        if !self.resolves {
            return Err(KeyLookupError::MissingKey {
                key: presented.to_owned(),
            });
        }
        let auth = UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new()))
            .map_err(|error| KeyLookupError::Row(Box::new(error)))?;
        Ok(KeyLookupOutcome {
            auth,
            hashed_token: presented.to_owned(),
        })
    }
}
