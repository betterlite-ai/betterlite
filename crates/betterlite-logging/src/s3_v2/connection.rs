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
    Connection {
        bucket_name: resolve_field(source.bucket_name.as_ref(), defaults.bucket_name.as_ref()),
        region_name: resolve_field(source.region_name.as_ref(), defaults.region_name.as_ref()),
        api_version: resolve_field(source.api_version.as_ref(), defaults.api_version.as_ref()),
        endpoint_url: resolve_field(source.endpoint_url.as_ref(), defaults.endpoint_url.as_ref()),
        path: resolve_field(source.path.as_ref(), defaults.path.as_ref()),
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
    }
}

impl<Config> From<(&Source<Config>, &Source<Config>)> for Credentials
where
    Config: Default,
{
    fn from((source, defaults): (&Source<Config>, &Source<Config>)) -> Self {
        Self {
            aws_access_key_id: resolve_field(
                source.aws_access_key_id.as_ref(),
                defaults.aws_access_key_id.as_ref(),
            ),
            aws_secret_access_key: resolve_field(
                source.aws_secret_access_key.as_ref(),
                defaults.aws_secret_access_key.as_ref(),
            ),
            aws_session_token: resolve_field(
                source.aws_session_token.as_ref(),
                defaults.aws_session_token.as_ref(),
            ),
            aws_session_name: resolve_field(
                source.aws_session_name.as_ref(),
                defaults.aws_session_name.as_ref(),
            ),
            aws_profile_name: resolve_field(
                source.aws_profile_name.as_ref(),
                defaults.aws_profile_name.as_ref(),
            ),
            aws_role_name: resolve_field(
                source.aws_role_name.as_ref(),
                defaults.aws_role_name.as_ref(),
            ),
            aws_web_identity_token: resolve_field(
                source.aws_web_identity_token.as_ref(),
                defaults.aws_web_identity_token.as_ref(),
            ),
            aws_sts_endpoint: resolve_field(
                source.aws_sts_endpoint.as_ref(),
                defaults.aws_sts_endpoint.as_ref(),
            ),
        }
    }
}

trait Blank {
    fn is_blank(&self) -> bool;
}

impl Blank for String {
    fn is_blank(&self) -> bool {
        self.is_empty()
    }
}

impl Blank for SecretString {
    fn is_blank(&self) -> bool {
        self.expose_secret().is_empty()
    }
}

fn resolve_field<Value>(source: Option<&Value>, fallback: Option<&Value>) -> Option<Value>
where
    Value: Blank + Clone,
{
    match source {
        Some(value) if !value.is_blank() => Some(value.clone()),
        _ => fallback.cloned(),
    }
}
