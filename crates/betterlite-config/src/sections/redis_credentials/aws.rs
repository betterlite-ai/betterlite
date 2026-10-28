use crate::loader::{ConfigError, EnvironmentLookup};

use super::{RawSettings, coerces_to_true};

const ELASTICACHE_TOKEN_TTL_SECONDS: u64 = 900;

#[derive(Debug, Clone)]
pub struct AwsSettings {
    pub user_name: String,
    pub cache_name: String,
    pub region: String,
    pub serverless: bool,
    pub token_ttl_seconds: u64,
}

pub(super) fn build_aws<L>(
    settings: &RawSettings,
    lookup: &L,
) -> Result<Option<AwsSettings>, ConfigError>
where
    L: EnvironmentLookup,
{
    let region = settings
        .aws_iam_region
        .clone()
        .or_else(|| lookup.lookup("AWS_REGION"))
        .or_else(|| lookup.lookup("AWS_DEFAULT_REGION"));
    let missing = [
        ("aws_iam_user_name", settings.aws_iam_user_name.as_ref()),
        ("aws_iam_cache_name", settings.aws_iam_cache_name.as_ref()),
        ("aws_iam_region", region.as_ref()),
    ];
    if let Some((name, None)) = missing.into_iter().find(|(_, value)| value.is_none()) {
        return Err(ConfigError::MissingEnvironment {
            variable: name.to_owned(),
            path: format!("litellm_settings.{name}"),
        });
    }
    if !settings.ssl.as_ref().is_some_and(coerces_to_true) {
        return Err(ConfigError::InvalidScalar {
            path: "litellm_settings.ssl".to_owned(),
            value: false,
        });
    }
    Ok(Some(AwsSettings {
        user_name: settings.aws_iam_user_name.clone().unwrap_or_default(),
        cache_name: settings
            .aws_iam_cache_name
            .clone()
            .unwrap_or_default()
            .to_ascii_lowercase(),
        region: region.unwrap_or_default(),
        serverless: settings
            .aws_iam_serverless
            .as_ref()
            .is_some_and(coerces_to_true),
        token_ttl_seconds: ELASTICACHE_TOKEN_TTL_SECONDS,
    }))
}
