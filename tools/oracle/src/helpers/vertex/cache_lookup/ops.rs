use std::collections::BTreeMap;

use betterlite_vertex::cache_lookup::target::{CacheTargetInput, cache_target};
use betterlite_vertex::endpoint_target::target::AuthHeader;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde_json::{Value, json};

use super::input::{Input, TargetCase};

#[derive(Serialize)]
#[serde(untagged)]
enum AuthView {
    HeaderMap(BTreeMap<String, String>),
    Token(String),
    Absent,
}

impl From<Option<&AuthHeader>> for AuthView {
    fn from(auth: Option<&AuthHeader>) -> Self {
        match auth {
            None => Self::Absent,
            Some(AuthHeader::Token(token)) => Self::Token(token.expose_secret().to_owned()),
            Some(AuthHeader::Headers(headers)) => Self::HeaderMap(
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
                    .collect(),
            ),
        }
    }
}

pub(crate) fn run(input: &Input) -> Value {
    let mut rows = Vec::with_capacity(input.targets.len());
    rows.extend(input.targets.iter().map(target_row));
    json!({ "targets": rows })
}

fn target_row(case: &TargetCase) -> Value {
    let token = case
        .auth
        .as_ref()
        .map(|value| SecretString::from(value.clone()));
    let key = case
        .gemini_api_key
        .as_ref()
        .map(|value| SecretString::from(value.clone()));
    let input = CacheTargetInput {
        provider: case.provider.provider(),
        api_base: case.api_base.as_deref(),
        project: case.project.as_deref(),
        location: case.location.as_deref(),
        auth_token: token.as_ref(),
        gemini_api_key: key.as_ref(),
        model: case.model.as_deref(),
    };
    match cache_target(&input) {
        Ok(target) => {
            let auth = AuthView::from(target.auth.as_ref());
            json!({"url": target.url, "auth": auth})
        }
        Err(_error) => json!({"error": true}),
    }
}
