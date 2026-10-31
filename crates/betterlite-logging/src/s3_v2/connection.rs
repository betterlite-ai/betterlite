use crate::s3_transport::option_value::OptionValue;
use crate::s3_v2::source::Source;
use secrecy::{ExposeSecret as _, SecretString};
use std::array;
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
    let [bucket_name, region_name, api_version, endpoint_url, path] = resolve_fields(
        [
            (&source.bucket_name, &defaults.bucket_name),
            (&source.region_name, &defaults.region_name),
            (&source.api_version, &defaults.api_version),
            (&source.endpoint_url, &defaults.endpoint_url),
            (&source.path, &defaults.path),
        ],
        String::is_empty,
    );
    Connection {
        bucket_name,
        region_name,
        api_version,
        endpoint_url,
        path,
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
        let [
            aws_access_key_id,
            aws_secret_access_key,
            aws_session_token,
            aws_web_identity_token,
        ] = resolve_credential_secrets(source, defaults);
        let [
            aws_session_name,
            aws_profile_name,
            aws_role_name,
            aws_sts_endpoint,
        ] = resolve_credential_strings(source, defaults);
        Self {
            aws_access_key_id,
            aws_secret_access_key,
            aws_session_token,
            aws_session_name,
            aws_profile_name,
            aws_role_name,
            aws_web_identity_token,
            aws_sts_endpoint,
        }
    }
}

fn resolve_credential_secrets<Config>(
    source: &Source<Config>,
    defaults: &Source<Config>,
) -> [Option<SecretString>; 4]
where
    Config: Default,
{
    let fields = [
        (&source.aws_access_key_id, &defaults.aws_access_key_id),
        (
            &source.aws_secret_access_key,
            &defaults.aws_secret_access_key,
        ),
        (&source.aws_session_token, &defaults.aws_session_token),
        (
            &source.aws_web_identity_token,
            &defaults.aws_web_identity_token,
        ),
    ];
    resolve_fields(fields, |value: &SecretString| {
        value.expose_secret().is_empty()
    })
}

fn resolve_credential_strings<Config>(
    source: &Source<Config>,
    defaults: &Source<Config>,
) -> [Option<String>; 4]
where
    Config: Default,
{
    let fields = [
        (&source.aws_session_name, &defaults.aws_session_name),
        (&source.aws_profile_name, &defaults.aws_profile_name),
        (&source.aws_role_name, &defaults.aws_role_name),
        (&source.aws_sts_endpoint, &defaults.aws_sts_endpoint),
    ];
    resolve_fields(fields, String::is_empty)
}

fn resolve_fields<Value, const N: usize>(
    fields: [(&Option<Value>, &Option<Value>); N],
    is_blank: impl Fn(&Value) -> bool,
) -> [Option<Value>; N]
where
    Value: Clone,
{
    let mut resolved: [Option<Value>; N] = array::from_fn(|_| None);
    for (slot, (provided, fallback)) in resolved.iter_mut().zip(fields) {
        *slot = provided
            .as_ref()
            .filter(|value| !is_blank(value))
            .or(fallback.as_ref())
            .cloned();
    }
    resolved
}
