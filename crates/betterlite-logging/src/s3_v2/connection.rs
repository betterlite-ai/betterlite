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
    pub use_ssl: Option<OptionValue>,
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
    let mut connection = Connection {
        bucket_name,
        region_name,
        api_version,
        endpoint_url,
        path,
        use_ssl: None,
        verify: None,
        config: None,
        credentials,
        log_prompts_only: None,
        partition_granularity: None,
    };
    apply_options(source, defaults, &mut connection);
    connection
}

fn apply_options<Config>(
    source: &Source<Config>,
    defaults: &Source<Config>,
    connection: &mut Connection<Config>,
) where
    Config: Clone + Default,
{
    connection.use_ssl = source
        .use_ssl
        .clone()
        .or_else(|| defaults.use_ssl.clone())
        .or(Some(OptionValue::Bool(true)));
    connection.verify = source.verify.clone().or_else(|| defaults.verify.clone());
    connection.config = source
        .config
        .as_ref()
        .filter(|config| !config.is_empty())
        .or(defaults.config.as_ref())
        .cloned();
    connection.log_prompts_only = defaults
        .log_prompts_only
        .clone()
        .or_else(|| source.log_prompts_only.clone());
    connection.partition_granularity = defaults
        .partition_granularity
        .clone()
        .or_else(|| source.partition_granularity.clone());
}

impl<Config> From<(&Source<Config>, &Source<Config>)> for Credentials
where
    Config: Default,
{
    fn from((source, defaults): (&Source<Config>, &Source<Config>)) -> Self {
        let secrets = resolve_fields(
            [
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
            ],
            |value: &SecretString| value.expose_secret().is_empty(),
        );
        let strings = resolve_fields(
            [
                (&source.aws_session_name, &defaults.aws_session_name),
                (&source.aws_profile_name, &defaults.aws_profile_name),
                (&source.aws_role_name, &defaults.aws_role_name),
                (&source.aws_sts_endpoint, &defaults.aws_sts_endpoint),
            ],
            String::is_empty,
        );
        let [access_key, secret_key, session_token, web_identity_token] = secrets;
        let [session_name, profile_name, role_name, sts_endpoint] = strings;
        Self {
            aws_access_key_id: access_key,
            aws_secret_access_key: secret_key,
            aws_session_token: session_token,
            aws_session_name: session_name,
            aws_profile_name: profile_name,
            aws_role_name: role_name,
            aws_web_identity_token: web_identity_token,
            aws_sts_endpoint: sts_endpoint,
        }
    }
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
