use std::future::{Ready, ready};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::request_policy::{Params, RequestValue};

pub(super) const ID: Uuid = Uuid::from_u128(0x1111_1111_2222_4333_8444_5555_5555_5555);
pub(super) const ENV_ACCESS_KEY: &str = "AKIDEXAMPLE";

pub(super) fn params(pairs: &[(&str, &str)]) -> Params {
    let mut built = Params::new();
    for (name, value) in pairs {
        built.insert((*name).to_owned(), RequestValue::Text((*value).to_owned()));
    }
    built
}

pub(super) fn rendered(params: &Params) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for (name, value) in params {
        let text = match value {
            RequestValue::Text(text) => text.clone(),
            RequestValue::Raw(raw) => format!("raw {raw}"),
            RequestValue::Secret(secret) => format!("secret {}", secret.expose_secret()),
            RequestValue::Mapping(mapping) => format!("mapping {:?}", rendered(mapping)),
        };
        pairs.push((name.clone(), text));
    }
    pairs
}

// the credential chain stand-in: request keys sign as given, otherwise the environment identity answers
pub(super) fn chain_credentials(
    settings: &AwsSettings,
    _region: &str,
) -> Ready<Result<Credentials, CredentialError>> {
    let credentials = match (&settings.access_key_id, &settings.secret_access_key) {
        (Some(access_key_id), Some(secret_access_key)) => Credentials {
            access_key_id: access_key_id.clone(),
            secret_access_key: secret_access_key.clone(),
            session_token: settings.session_token.clone(),
        },
        (Some(_) | None, _) => Credentials {
            access_key_id: SecretString::from(ENV_ACCESS_KEY.to_owned()),
            secret_access_key: SecretString::from("secret".to_owned()),
            session_token: None,
        },
    };
    ready(Ok(credentials))
}
