use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_bedrock::aws_settings::tls::resolve_tls;
use betterlite_bedrock::aws_settings::{
    assume_role_parameters, bedrock_bearer_token, canonicalize_tags, pop_aws_auth_params,
    s3_static_key_pair,
};
use betterlite_bedrock::aws_settings::{
    credential_cache::cache_key, error::aws_auth_error_projection, policy::web_identity_policy,
    sources::auth_selector_presence,
};
use betterlite_config::sections::{
    bedrock_aws_settings::AwsS3Settings, bedrock_settings::AwsSettings,
};
use input::Input;
use secrecy::ExposeSecret as _;
use serde_json::json;
use serde_json::{Value, value::RawValue};
use std::{collections::BTreeMap, fmt::Display};
use tokio::runtime::Builder;

mod input;
mod io;
mod native;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(failure)?;
        match input {
            Input::StaticKeyPair(value) => Ok(pair(value)),
            Input::CanonicalTags(tags) => {
                Ok(json!({"value": canonicalize_tags(tags.tags.clone()).map_err(failure)?}))
            }
            Input::AssumeRole(role) => Ok(
                json!({"value": assume_role_parameters(role.role_arn.clone(), role.session_name.clone(), role.external_id.clone(), canonicalize_tags(role.tags.clone()).map_err(failure)?)}),
            ),
            Input::WebPolicy => Ok(json!({"value": web_identity_policy()})),
            Input::Bearer(token) => Ok(
                json!({"value": bedrock_bearer_token(token.api_key.as_ref(), token.configured_token.as_ref()).map(|value| value.expose_secret().to_owned())}),
            ),
            Input::PopAuth(params) => Ok(pop(params)),
            Input::AuthSelector(config) => Ok(selector(config)),
            Input::AuthError(error) => auth_error(error),
            Input::CacheKey(keys) => {
                let left = cache_key(&keys.left).map_err(failure)?;
                Ok(
                    json!({"length": left.len(), "equal": left == cache_key(&keys.right).map_err(failure)?}),
                )
            }
            Input::Cache(cache) => runtime.block_on(io::cache(cache)),
            Input::Local(local) => runtime.block_on(native::local(local)),
        }
    }
}

fn pop(params: &BTreeMap<String, Box<RawValue>>) -> Value {
    let mut remaining = params.clone();
    let result = pop_aws_auth_params(&mut remaining);
    json!({"remaining": remaining, "valid": result.is_ok()})
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "AwsSettingsError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn auth_error(input: &input::AuthError) -> HelperResult {
    use betterlite_config::{loader::ConfigLoader, tls::Verification};
    let environment = BTreeMap::new();
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load("{}")
    .map_err(failure)?;
    let policy = resolve_tls(&config, &environment, input.ssl_verify.as_ref()).map_err(failure)?;
    let verification = match policy.verification {
        Verification::Enabled => json!(true),
        Verification::Disabled => json!(false),
        Verification::CaFile(path) => json!(path),
    };
    Ok(
        json!({"error": aws_auth_error_projection(input.status, input.message.clone()), "ssl_verify": verification}),
    )
}

fn pair(input: &input::Pair) -> Value {
    let settings = AwsS3Settings {
        access_key_id: input.s3_access_key_id.clone(),
        secret_access_key: input.s3_secret_access_key.clone(),
    };
    json!({"value": s3_static_key_pair(&settings).map(|(access, secret)| [access.expose_secret().to_owned(), secret.expose_secret().to_owned()])})
}

fn selector(input: &input::Config) -> Value {
    let [web_identity, role, profile, session_token, static_pair] =
        auth_selector_presence(&AwsSettings::from(input));
    json!({"web_identity": web_identity, "role": role, "profile": profile, "session_token": session_token, "static": static_pair})
}
