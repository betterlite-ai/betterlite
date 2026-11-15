use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_mcp::credential_contracts::auth_config::id_jag::{
    ClientAuth, ClientSecretAuth, PrivateKeyJwtAuth,
};
use betterlite_server::mcp::outbound_credentials::token_endpoint::TokenEndpointClient;
use betterlite_server::mcp::outbound_credentials::token_endpoint::post::{
    PostedForm, TokenEndpointPost,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::Value;

use super::support::{cred_tag, runtime, value_error};
use crate::helpers::HelperResult;

#[derive(Deserialize)]
struct FetchInput {
    endpoint: String,
    client_id: String,
    #[serde(default)]
    grant: BTreeMap<String, String>,
    client_auth: AuthInput,
    response: ResponseInput,
}

#[derive(Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
enum AuthInput {
    ClientSecret {
        client_secret: String,
    },
    PrivateKeyJwt {
        private_key: String,
        key_id: Option<String>,
        signing_alg: String,
    },
}

impl AuthInput {
    fn client_auth(self) -> ClientAuth {
        match self {
            AuthInput::ClientSecret { client_secret } => {
                ClientAuth::ClientSecret(ClientSecretAuth {
                    client_secret: SecretString::from(client_secret),
                })
            }
            AuthInput::PrivateKeyJwt {
                private_key,
                key_id,
                signing_alg,
            } => ClientAuth::PrivateKeyJwt(PrivateKeyJwtAuth {
                private_key: SecretString::from(private_key),
                key_id,
                signing_alg,
            }),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ResponseInput {
    Answered { status: u16, body: String },
    Unreachable,
}

struct ScriptedPost {
    response: ResponseInput,
    form: Mutex<Option<Vec<(String, String)>>>,
}

#[async_trait]
impl TokenEndpointPost for ScriptedPost {
    async fn post(&self, _endpoint: &str, form: &[(String, SecretString)]) -> PostedForm {
        let exposed: Vec<(String, String)> = form
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        *self.form.lock().unwrap_or_else(PoisonError::into_inner) = Some(exposed);
        match &self.response {
            ResponseInput::Answered { status, body } => PostedForm::Answered {
                status: *status,
                body: body.clone().into_bytes(),
            },
            ResponseInput::Unreachable => PostedForm::Unreachable {
                kind: "ConnectError",
            },
        }
    }
}

pub(super) fn fetch(input: &Value) -> HelperResult {
    let parsed: FetchInput =
        serde_json::from_value(input.clone()).map_err(|error| value_error(error.to_string()))?;
    let FetchInput {
        endpoint,
        client_id,
        grant,
        client_auth,
        response,
    } = parsed;
    let auth = client_auth.client_auth();
    let client = TokenEndpointClient::new(ScriptedPost {
        response,
        form: Mutex::new(None),
    });
    let outcome =
        runtime()?.block_on(async { client.fetch(&endpoint, &client_id, &grant, &auth).await });
    let result = match &outcome {
        Ok(token) => serde_json::json!({
            "ok": true,
            "access_token": token.access_token,
            "expires_in": token.expires_in,
        }),
        Err(error) => serde_json::json!({"ok": false, "error": cred_tag(error)}),
    };
    let form = client
        .post
        .form
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(form_view);
    Ok(serde_json::json!({"result": result, "form": form}))
}

fn form_view(pairs: &[(String, String)]) -> Value {
    let mut rendered: Vec<[String; 2]> = pairs
        .iter()
        .map(|(name, value)| {
            let shown = if name == "client_assertion" {
                "<assertion>".to_owned()
            } else {
                value.clone()
            };
            [name.clone(), shown]
        })
        .collect();
    rendered.sort();
    serde_json::json!(rendered)
}
