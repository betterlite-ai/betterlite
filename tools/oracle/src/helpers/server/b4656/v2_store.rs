use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_server::mcp::outbound_credentials::v2_store::{
    CredentialPayload, CredentialReader, V2PerUserTokenStore,
};
use betterlite_server::mcp::outbound_credentials::{OAuthTokenStore as _, TokenStoreUnavailable};
use serde::Deserialize;

use super::runtime;
use super::support::result_view;
use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default = "default_user_id")]
    pub(super) user_id: String,
    pub(super) payloads: Vec<Option<CredentialPayload>>,
}

fn default_user_id() -> String {
    "user".to_owned()
}

struct ScriptedReader {
    payloads: Mutex<VecDeque<Option<CredentialPayload>>>,
}

#[async_trait]
impl CredentialReader for ScriptedReader {
    async fn read_credential(
        &self,
        _user_id: &str,
        _server_id: &str,
    ) -> Result<Option<CredentialPayload>, TokenStoreUnavailable> {
        Ok(self
            .payloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .flatten())
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    let store = V2PerUserTokenStore {
        reader: ScriptedReader {
            payloads: Mutex::new(VecDeque::from(input.payloads.clone())),
        },
    };
    runtime()?.block_on(async {
        let mut results = Vec::new();
        for _ in &input.payloads {
            let result = store.fetch(&input.user_id, "server").await;
            results.push(result_view(result));
        }
        Ok(serde_json::json!({"results": results}))
    })
}
