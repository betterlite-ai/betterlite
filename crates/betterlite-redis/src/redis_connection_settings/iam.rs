mod tests;

use secrecy::ExposeSecret;

use super::coercion::coerce_bool;
use super::environment::EnvironmentLookup;
use super::error::RedisSettingsError;
use super::model::scalars::{BoolOrText, PassthroughValue};
use super::model::settings::{RedisConnectionSettings, RedisOverrides};
use super::url_parts::uses_tls;

// litellm._redis._get_redis_client_logic resolves GCP IAM, then Azure AD, then AWS ElastiCache IAM, each only when a higher-precedence provider did not already resolve and the caller did not already set an explicit credential_provider.
pub fn apply_iam_precedence<E>(
    overrides: &RedisOverrides,
    settings: &mut RedisConnectionSettings,
    env: &E,
) -> Result<(), RedisSettingsError>
where
    E: EnvironmentLookup,
{
    if settings.credential_provider {
        return Ok(());
    }

    let gcp_active = resolve_string(
        overrides.gcp_service_account.as_deref(),
        env,
        "REDIS_GCP_SERVICE_ACCOUNT",
    )
    .is_some();
    if gcp_active {
        apply_gcp(overrides, settings, env);
        return Ok(());
    }

    let azure_ad_token = resolve_string(
        overrides
            .azure_redis_ad_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        env,
        "REDIS_AZURE_REDIS_AD_TOKEN",
    )
    .or_else(|| env.lookup("REDIS_AZURE_AD_TOKEN"));
    let azure_ad_enabled = azure_ad_token.is_some_and(|token| token.eq_ignore_ascii_case("true"));
    if azure_ad_enabled {
        return Ok(());
    }

    let aws_iam_enabled = coerce_bool(
        overrides
            .aws_iam_auth
            .clone()
            .or_else(|| env.lookup("REDIS_AWS_IAM_AUTH").map(BoolOrText::Text)),
    )
    .unwrap_or(false);
    if aws_iam_enabled {
        apply_aws(overrides, settings, env)?;
    }
    Ok(())
}

fn apply_gcp<E>(overrides: &RedisOverrides, settings: &mut RedisConnectionSettings, env: &E)
where
    E: EnvironmentLookup,
{
    let ssl_ca_certs = resolve_string(
        overrides.gcp_ssl_ca_certs.as_deref(),
        env,
        "REDIS_GCP_SSL_CA_CERTS",
    );
    if let Some(resolved) = ssl_ca_certs
        && settings.ssl == Some(true)
    {
        settings
            .passthrough
            .insert("ssl_ca_certs".to_owned(), PassthroughValue::Text(resolved));
    }
}

fn apply_aws<E>(
    overrides: &RedisOverrides,
    settings: &mut RedisConnectionSettings,
    env: &E,
) -> Result<(), RedisSettingsError>
where
    E: EnvironmentLookup,
{
    let tls = uses_tls(
        settings.startup_nodes.is_some(),
        settings.ssl,
        settings.url.as_ref().map(ExposeSecret::expose_secret),
    );
    if !tls {
        return Err(RedisSettingsError::AwsIamRequiresTls);
    }

    let user_name = resolve_string(
        overrides.aws_iam_user_name.as_deref(),
        env,
        "REDIS_AWS_IAM_USER_NAME",
    );
    let cache_name = resolve_string(
        overrides.aws_iam_cache_name.as_deref(),
        env,
        "REDIS_AWS_IAM_CACHE_NAME",
    );
    let region = resolve_string(
        overrides.aws_iam_region.as_deref(),
        env,
        "REDIS_AWS_IAM_REGION",
    )
    .or_else(|| env.lookup("AWS_REGION"))
    .or_else(|| env.lookup("AWS_DEFAULT_REGION"));

    let mut missing = Vec::new();
    if user_name.is_none() {
        missing.push("aws_iam_user_name".to_owned());
    }
    if cache_name.is_none() {
        missing.push("aws_iam_cache_name".to_owned());
    }
    if region.is_none() {
        missing.push("aws_iam_region".to_owned());
    }
    if !missing.is_empty() {
        return Err(RedisSettingsError::AwsIamMissingSettings { missing });
    }

    settings.credential_provider = true;
    Ok(())
}

fn resolve_string<E>(explicit: Option<&str>, env: &E, var: &str) -> Option<String>
where
    E: EnvironmentLookup,
{
    if let Some(text) = explicit.filter(|text| !text.is_empty()) {
        return Some(text.to_owned());
    }
    env.lookup(var).filter(|text| !text.is_empty())
}
