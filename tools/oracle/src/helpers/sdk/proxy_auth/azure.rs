use std::convert::Infallible;
use std::future::{Future, ready};

use betterlite_sdk::proxy_auth::{AccessToken, AzureADCredential, TokenCredential};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::OnceCell;

use crate::helpers::HelperResult;

const EXPIRES_ON: i64 = 5_000;

#[derive(Debug, Deserialize)]
pub(crate) struct Azure {
    installed: bool,
    injected: bool,
    scopes: Vec<String>,
}

struct Named(&'static str);

impl TokenCredential for Named {
    type Error = Infallible;

    fn get_token(
        &self,
        scope: &str,
    ) -> impl Future<Output = Result<AccessToken, Infallible>> + Send {
        ready(Ok(AccessToken {
            token: SecretString::from(format!("{}-{scope}", self.0)),
            expires_on: EXPIRES_ON,
        }))
    }
}

pub(super) fn run(input: &Azure) -> HelperResult {
    let build: fn() -> Named = || Named("chain");
    let credential = AzureADCredential {
        credential: if input.injected {
            OnceCell::new_with(Some(Named("injected")))
        } else {
            OnceCell::new()
        },
        default_chain: input.installed.then_some(build),
    };
    let chains_built_before = usize::from(!input.injected && credential.credential.initialized());
    let results: Vec<Value> = super::runtime()?.block_on(async {
        let mut results = Vec::new();
        for scope in &input.scopes {
            results.push(match credential.get_token(scope).await {
                Ok(token) => json!({
                    "token": token.token.expose_secret(),
                    "expires_on": token.expires_on,
                }),
                Err(error) => json!({"error": error.to_string()}),
            });
        }
        results
    });
    Ok(json!({
        "chains_built_before": chains_built_before,
        "chains_built": usize::from(!input.injected && credential.credential.initialized()),
        "results": results,
    }))
}
