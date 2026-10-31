mod tests;

use secrecy::{ExposeSecret, SecretString};

use super::coercion::coerce_bool;
use super::environment::EnvironmentLookup;
use super::error::RedisSettingsError;
use super::model::scalars::{BoolOrText, PassthroughValue};
use super::model::settings::{CredentialProviderSettings, RedisConnectionSettings, RedisOverrides};
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

    let service_account = resolve_string(
        overrides.gcp_service_account.as_deref(),
        env,
        "REDIS_GCP_SERVICE_ACCOUNT",
    );
    if let Some(account) = service_account {
        apply_gcp(account, overrides, settings, env);
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
        settings.credential_provider_settings = Some(azure_provider(overrides, env));
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

fn apply_gcp<E>(
    service_account: String,
    overrides: &RedisOverrides,
    settings: &mut RedisConnectionSettings,
    env: &E,
) where
    E: EnvironmentLookup,
{
    let ssl_ca_certs = resolve_string(
        overrides.gcp_ssl_ca_certs.as_deref(),
        env,
        "REDIS_GCP_SSL_CA_CERTS",
    );
    if let Some(resolved) = ssl_ca_certs.as_ref()
        && settings.ssl == Some(true)
    {
        settings.passthrough.insert(
            "ssl_ca_certs".to_owned(),
            PassthroughValue::Text(resolved.clone()),
        );
    }
    settings.credential_provider_settings = Some(CredentialProviderSettings::Gcp {
        service_account,
        ssl_ca_certs,
    });
}

fn azure_provider<E>(overrides: &RedisOverrides, env: &E) -> CredentialProviderSettings
where
    E: EnvironmentLookup,
{
    let client_id = resolve_string(overrides.azure_client_id.as_deref(), env, "AZURE_CLIENT_ID");
    let tenant_id = resolve_string(overrides.azure_tenant_id.as_deref(), env, "AZURE_TENANT_ID");
    let client_secret = resolve_string(
        overrides
            .azure_client_secret
            .as_ref()
            .map(ExposeSecret::expose_secret),
        env,
        "AZURE_CLIENT_SECRET",
    )
    .map(SecretString::from);
    CredentialProviderSettings::Azure {
        client_id,
        tenant_id,
        client_secret,
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
    if !uses_tls(
        settings.startup_nodes.is_some(),
        settings.ssl,
        settings.url.as_ref().map(ExposeSecret::expose_secret),
    ) {
        return Err(RedisSettingsError::AwsIamRequiresTls);
    }

    settings.credential_provider_settings = Some(aws_provider(overrides, env)?);
    settings.credential_provider = true;
    Ok(())
}

fn aws_provider<E>(
    overrides: &RedisOverrides,
    env: &E,
) -> Result<CredentialProviderSettings, RedisSettingsError>
where
    E: EnvironmentLookup,
{
    let fields = [
        (
            "REDIS_AWS_IAM_USER_NAME",
            overrides.aws_iam_user_name.as_deref(),
        ),
        (
            "REDIS_AWS_IAM_CACHE_NAME",
            overrides.aws_iam_cache_name.as_deref(),
        ),
        ("REDIS_AWS_IAM_REGION", overrides.aws_iam_region.as_deref()),
    ];
    let [user_name, cache_name, configured_region] =
        fields.map(|(name, explicit)| resolve_string(explicit, env, name));
    let region = configured_region
        .or_else(|| env.lookup("AWS_REGION"))
        .or_else(|| env.lookup("AWS_DEFAULT_REGION"));

    let (user, cache, location) =
        required_aws_fields(user_name.as_ref(), cache_name.as_ref(), region.as_ref())?;
    Ok(CredentialProviderSettings::Aws {
        user_name: user,
        cache_name: cache,
        region: location,
        serverless: coerce_bool(
            overrides
                .aws_iam_serverless
                .clone()
                .or_else(|| env.lookup("REDIS_AWS_IAM_SERVERLESS").map(BoolOrText::Text)),
        )
        .unwrap_or(false),
    })
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

fn required_aws_fields(
    user_name: Option<&String>,
    cache_name: Option<&String>,
    region: Option<&String>,
) -> Result<(String, String, String), RedisSettingsError> {
    let (Some(user), Some(cache), Some(location)) = (user_name, cache_name, region) else {
        let missing = [
            ("aws_iam_user_name", &user_name),
            ("aws_iam_cache_name", &cache_name),
            ("aws_iam_region", &region),
        ]
        .into_iter()
        .filter(|(_, value)| value.is_none())
        .map(|(name, _)| name.to_owned())
        .collect();
        return Err(RedisSettingsError::AwsIamMissingSettings { missing });
    };
    Ok((user.clone(), cache.clone(), location.clone()))
}
