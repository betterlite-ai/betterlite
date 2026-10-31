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
        apply_azure(overrides, settings, env);
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
    let service_account = resolve_string(
        overrides.gcp_service_account.as_deref(),
        env,
        "REDIS_GCP_SERVICE_ACCOUNT",
    );
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
    let provider = service_account.map(|service_account| CredentialProviderSettings::Gcp {
        service_account,
        ssl_ca_certs,
    });
    set_provider(settings, provider);
}

fn apply_azure<E>(overrides: &RedisOverrides, settings: &mut RedisConnectionSettings, env: &E)
where
    E: EnvironmentLookup,
{
    let provider = azure_provider(overrides, env);
    set_provider(settings, provider);
}

fn azure_provider<E>(overrides: &RedisOverrides, env: &E) -> Option<CredentialProviderSettings>
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
    Some(CredentialProviderSettings::Azure {
        client_id,
        tenant_id,
        client_secret,
    })
}

fn set_provider(
    settings: &mut RedisConnectionSettings,
    provider: Option<CredentialProviderSettings>,
) {
    settings.credential_provider = true;
    settings.credential_provider_settings = provider;
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
    let Some(user_name) = user_name else {
        return Err(RedisSettingsError::AwsIamMissingSettings {
            missing: vec!["aws_iam_user_name".to_owned()],
        });
    };
    let Some(cache_name) = cache_name else {
        return Err(RedisSettingsError::AwsIamMissingSettings {
            missing: vec!["aws_iam_cache_name".to_owned()],
        });
    };
    let Some(region) = region else {
        return Err(RedisSettingsError::AwsIamMissingSettings {
            missing: vec!["aws_iam_region".to_owned()],
        });
    };
    settings.credential_provider_settings = Some(CredentialProviderSettings::Aws {
        user_name,
        cache_name,
        region,
        serverless: coerce_bool(
            overrides
                .aws_iam_serverless
                .clone()
                .or_else(|| env.lookup("REDIS_AWS_IAM_SERVERLESS").map(BoolOrText::Text)),
        )
        .unwrap_or(false),
    });
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
