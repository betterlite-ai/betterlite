use std::collections::BTreeMap;
use std::future::Future;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::tls::VerifySetting;
use secrecy::SecretString;

use crate::aws_settings::AwsSessionTag;
use crate::aws_settings::credential_cache::{CacheError, CredentialCache, Parameter};
use crate::aws_settings::sources::{Credentials, ResolvedCredentials, SourceError};

use super::error::CredentialError;

pub(super) fn parameters(
    settings: &AwsSettings,
    tags: Option<&[AwsSessionTag]>,
    ssl_verify: Option<&VerifySetting>,
) -> BTreeMap<String, Parameter> {
    let secrets = [
        ("aws_access_key_id", &settings.access_key_id),
        ("aws_secret_access_key", &settings.secret_access_key),
        ("aws_session_token", &settings.session_token),
        ("aws_web_identity_token", &settings.web_identity_token),
    ];
    let texts = [
        ("aws_region_name", &settings.region_name),
        ("aws_session_name", &settings.session_name),
        ("aws_profile_name", &settings.profile_name),
        ("aws_role_name", &settings.role_name),
        ("aws_sts_endpoint", &settings.sts_endpoint),
        ("aws_external_id", &settings.external_id),
    ];
    let mut parameters = BTreeMap::new();
    for (name, secret) in secrets {
        let encoded = secret
            .as_ref()
            .map_or(Parameter::Null, |value| Parameter::Text(value.clone()));
        parameters.insert(name.to_owned(), encoded);
    }
    for (name, value) in texts {
        let encoded = value.as_ref().map_or(Parameter::Null, |text| {
            Parameter::Text(SecretString::from(text.clone()))
        });
        parameters.insert(name.to_owned(), encoded);
    }
    parameters.insert(
        "aws_session_tags".to_owned(),
        tags.map_or(Parameter::Null, |values| Parameter::Tags(values.to_vec())),
    );
    parameters.insert("ssl_verify".to_owned(), ssl_parameter(ssl_verify));
    parameters
}

fn ssl_parameter(ssl_verify: Option<&VerifySetting>) -> Parameter {
    match ssl_verify {
        None => Parameter::Null,
        Some(VerifySetting::Bool(value)) => Parameter::Boolean(*value),
        Some(VerifySetting::Path(path)) => Parameter::Text(SecretString::from(path.clone())),
    }
}

pub(super) async fn cached<F, Fut>(
    cache: &CredentialCache,
    parameters: &BTreeMap<String, Parameter>,
    fetch: F,
) -> Result<Credentials, CredentialError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<ResolvedCredentials, CredentialError>>,
{
    let mut failure: Option<CredentialError> = None;
    // the shared cache's fetcher carries SourceError, so a typed failure returns through this slot
    let result = cache
        .get_or_fetch(parameters, || {
            let slot = &mut failure;
            async move {
                match fetch().await {
                    Ok(resolved) => Ok(resolved),
                    Err(error) => {
                        *slot = Some(error);
                        Err(SourceError::MissingProvider)
                    }
                }
            }
        })
        .await;
    match result {
        Ok(credentials) => Ok(credentials),
        Err(CacheError::Source(_)) => {
            failure.map_or_else(|| Err(CredentialError::Cache(CacheError::Unavailable)), Err)
        }
        Err(error) => Err(CredentialError::Cache(error)),
    }
}
