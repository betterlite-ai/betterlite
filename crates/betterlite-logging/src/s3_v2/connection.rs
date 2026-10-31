use crate::s3_transport::option_value::OptionValue;
use crate::s3_v2::source::Source;
use secrecy::{ExposeSecret as _, SecretString};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Connection<Config> {
    pub bucket_name: Option<String>,
    pub region_name: Option<String>,
    pub api_version: Option<String>,
    pub endpoint_url: Option<String>,
    pub path: Option<String>,
    pub verify: Option<OptionValue>,
    pub config: Option<BTreeMap<String, Config>>,
    pub credentials: Credentials,
    pub log_prompts_only: Option<OptionValue>,
    pub partition_granularity: Option<OptionValue>,
}

#[derive(Clone, Debug)]
pub struct Credentials {
    pub aws_access_key_id: Option<SecretString>,
    pub aws_secret_access_key: Option<SecretString>,
    pub aws_session_token: Option<SecretString>,
    pub aws_session_name: Option<String>,
    pub aws_profile_name: Option<String>,
    pub aws_role_name: Option<String>,
    pub aws_web_identity_token: Option<SecretString>,
    pub aws_sts_endpoint: Option<String>,
}

pub(super) fn resolve<Config>(
    source: &Source<Config>,
    defaults: &Source<Config>,
) -> Connection<Config>
where
    Config: Clone + Default,
{
    let credentials = Credentials::from((source, defaults));
    let mut connection = Connection {
        bucket_name: source.bucket_name.clone(),
        region_name: source.region_name.clone(),
        api_version: source.api_version.clone(),
        endpoint_url: source.endpoint_url.clone(),
        path: source.path.clone(),
        credentials,
        verify: source.verify.as_ref().or(defaults.verify.as_ref()).cloned(),
        config: source
            .config
            .as_ref()
            .filter(|config| !config.is_empty())
            .or(defaults.config.as_ref())
            .cloned(),
        log_prompts_only: defaults
            .log_prompts_only
            .as_ref()
            .or(source.log_prompts_only.as_ref())
            .cloned(),
        partition_granularity: defaults
            .partition_granularity
            .as_ref()
            .or(source.partition_granularity.as_ref())
            .cloned(),
    };
    for (field, fallback) in [
        (&mut connection.bucket_name, defaults.bucket_name.as_ref()),
        (&mut connection.region_name, defaults.region_name.as_ref()),
        (&mut connection.api_version, defaults.api_version.as_ref()),
        (&mut connection.endpoint_url, defaults.endpoint_url.as_ref()),
        (&mut connection.path, defaults.path.as_ref()),
    ] {
        if field.as_deref().is_none_or(str::is_empty) {
            *field = fallback.cloned();
        }
    }
    connection
}

impl<Config> From<(&Source<Config>, &Source<Config>)> for Credentials
where
    Config: Default,
{
    fn from((source, defaults): (&Source<Config>, &Source<Config>)) -> Self {
        let mut credentials = Self {
            aws_access_key_id: source.aws_access_key_id.clone(),
            aws_secret_access_key: source.aws_secret_access_key.clone(),
            aws_session_token: source.aws_session_token.clone(),
            aws_session_name: source.aws_session_name.clone(),
            aws_profile_name: source.aws_profile_name.clone(),
            aws_role_name: source.aws_role_name.clone(),
            aws_web_identity_token: source.aws_web_identity_token.clone(),
            aws_sts_endpoint: source.aws_sts_endpoint.clone(),
        };
        apply_secret_fallbacks(&mut credentials, defaults);
        apply_text_fallbacks(&mut credentials, defaults);
        credentials
    }
}

fn apply_secret_fallbacks<Config>(credentials: &mut Credentials, defaults: &Source<Config>)
where
    Config: Default,
{
    for (field, fallback) in [
        (
            &mut credentials.aws_access_key_id,
            defaults.aws_access_key_id.as_ref(),
        ),
        (
            &mut credentials.aws_secret_access_key,
            defaults.aws_secret_access_key.as_ref(),
        ),
        (
            &mut credentials.aws_session_token,
            defaults.aws_session_token.as_ref(),
        ),
        (
            &mut credentials.aws_web_identity_token,
            defaults.aws_web_identity_token.as_ref(),
        ),
    ] {
        if field
            .as_ref()
            .is_none_or(|value| value.expose_secret().is_empty())
        {
            *field = fallback.cloned();
        }
    }
}

fn apply_text_fallbacks<Config>(credentials: &mut Credentials, defaults: &Source<Config>)
where
    Config: Default,
{
    for (field, fallback) in [
        (
            &mut credentials.aws_session_name,
            defaults.aws_session_name.as_ref(),
        ),
        (
            &mut credentials.aws_profile_name,
            defaults.aws_profile_name.as_ref(),
        ),
        (
            &mut credentials.aws_role_name,
            defaults.aws_role_name.as_ref(),
        ),
        (
            &mut credentials.aws_sts_endpoint,
            defaults.aws_sts_endpoint.as_ref(),
        ),
    ] {
        if field.as_deref().is_none_or(str::is_empty) {
            *field = fallback.cloned();
        }
    }
}
