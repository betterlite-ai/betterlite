use crate::loader::{ConfigError, EnvironmentLookup};

use super::{RawSettings, ScalarBool, coerces_to_true, ssl_enabled};

const ELASTICACHE_TOKEN_TTL_SECONDS: u64 = 900;

#[derive(Debug, Clone)]
pub struct AwsSettings {
    pub user_name: String,
    pub cache_name: String,
    pub region: String,
    pub serverless: bool,
    pub token_ttl_seconds: u64,
}

impl AwsSettings {
    #[must_use]
    pub fn connect_query(&self) -> String {
        let mut pairs = vec![("Action", "connect"), ("User", self.user_name.as_str())];
        if self.serverless {
            pairs.push(("ResourceType", "ServerlessCache"));
        }
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish()
    }
}

pub(super) fn build_aws<L>(settings: &RawSettings, lookup: &L) -> Result<AwsSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    let region = settings
        .aws_iam_region
        .clone()
        .or_else(|| lookup.lookup("REDIS_AWS_IAM_REGION"))
        .or_else(|| lookup.lookup("AWS_REGION"))
        .or_else(|| lookup.lookup("AWS_DEFAULT_REGION"))
        .ok_or_else(|| missing_setting("aws_iam_region"))?;
    let user_name = settings
        .aws_iam_user_name
        .clone()
        .or_else(|| lookup.lookup("REDIS_AWS_IAM_USER_NAME"))
        .ok_or_else(|| missing_setting("aws_iam_user_name"))?;
    let cache_name = settings
        .aws_iam_cache_name
        .clone()
        .or_else(|| lookup.lookup("REDIS_AWS_IAM_CACHE_NAME"))
        .ok_or_else(|| missing_setting("aws_iam_cache_name"))?;
    let url = settings.url.clone().or_else(|| lookup.lookup("REDIS_URL"));
    if !uses_tls(url.as_deref(), ssl_enabled(settings, lookup)) {
        return Err(ConfigError::InvalidScalar {
            path: "litellm_settings.cache_params.ssl".to_owned(),
            value: false,
        });
    }
    let serverless = settings.aws_iam_serverless.as_ref().is_some_and(coerces_to_true)
        || lookup
            .lookup("REDIS_AWS_IAM_SERVERLESS")
            .is_some_and(|value| coerces_to_true(&ScalarBool::Text(value)));
    Ok(AwsSettings {
        user_name,
        cache_name: cache_name.to_ascii_lowercase(),
        region,
        serverless,
        token_ttl_seconds: ELASTICACHE_TOKEN_TTL_SECONDS,
    })
}

fn missing_setting(name: &'static str) -> ConfigError {
    ConfigError::MissingEnvironment {
        variable: name.to_owned(),
        path: format!("litellm_settings.cache_params.{name}"),
    }
}

fn uses_tls(url: Option<&str>, ssl_enabled: bool) -> bool {
    url.map_or(ssl_enabled, |value| {
        value
            .split_once("://")
            .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("rediss"))
    })
}
