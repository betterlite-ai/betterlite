use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use secrecy::SecretString;
use serde_json::{Value, json};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::bedrock_embedding_io::signed_get::sign_get_request;

use super::failure::Failure;
use super::input::SignedRequest;

pub(crate) fn run(case: &SignedRequest) -> Result<Value, Failure> {
    let now = UNIX_EPOCH
        .checked_add(Duration::from_secs(case.now))
        .ok_or(Failure::Clock)?;
    let signed = sign_get_request(
        &credentials(case),
        &case.url,
        &pairs(&case.headers),
        &case.region,
        now,
    )?;
    Ok(json!({
        "method": signed.method,
        "url": signed.url,
        "headers": signed.headers.into_iter().collect::<BTreeMap<String, String>>(),
    }))
}

fn credentials(case: &SignedRequest) -> Credentials {
    Credentials {
        access_key_id: SecretString::from(case.credentials.access_key_id.clone()),
        secret_access_key: SecretString::from(case.credentials.secret_access_key.clone()),
        session_token: case
            .credentials
            .session_token
            .clone()
            .map(SecretString::from),
    }
}

pub(crate) fn pairs(headers: &BTreeMap<String, String>) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}
