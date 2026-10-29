use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

use super::aws::build_aws;
use super::azure::resolve_azure;
use super::gcp::build_gcp;
use super::{Provider, ProviderWarning, RawSettings, config_or_environment_flag, non_empty};

pub(super) fn resolve_providers<L>(
    settings: &RawSettings,
    lookup: &L,
) -> Result<(Option<Provider>, Vec<ProviderWarning>), ConfigError>
where
    L: EnvironmentLookup,
{
    let mut warnings = Vec::new();
    let gcp = build_gcp(settings, lookup)?;

    let azure = resolve_provider(
        azure_ad_token_enabled(settings, lookup)?,
        &[(gcp.is_some(), ProviderWarning::GcpOutranksAzure)],
        &mut warnings,
        || resolve_azure(settings, lookup),
    )
    .transpose()?;

    let aws_iam_auth = config_or_environment_flag(
        settings.aws_iam_auth.as_ref(),
        lookup,
        "litellm_settings.cache_params.aws_iam_auth",
        "REDIS_AWS_IAM_AUTH",
    )?;
    let aws = resolve_provider(
        aws_iam_auth,
        &[
            (gcp.is_some(), ProviderWarning::GcpOutranksAws),
            (
                gcp.is_none() && azure.is_some(),
                ProviderWarning::AzureOutranksAws,
            ),
        ],
        &mut warnings,
        || build_aws(settings, lookup),
    )
    .transpose()?;

    let resolved = match (gcp, azure, aws) {
        (Some(provider), _, _) => Some(Provider::Gcp(provider)),
        (None, Some(provider), _) => Some(Provider::Azure(provider)),
        (None, None, Some(provider)) => Some(Provider::Aws(provider)),
        (None, None, None) => None,
    };

    Ok((resolved, warnings))
}

fn azure_ad_token_enabled<L>(settings: &RawSettings, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    let configured = non_empty(settings.azure_redis_ad_token.clone())
        .map(|value| {
            resolve_string(
                value,
                "litellm_settings.cache_params.azure_redis_ad_token",
                lookup,
            )
        })
        .transpose()?;
    let azure_ad_token = configured.or_else(|| lookup.lookup("REDIS_AZURE_AD_TOKEN"));
    Ok(azure_ad_token.is_some_and(|value| value.eq_ignore_ascii_case("true")))
}

fn resolve_provider<T, B>(
    enabled: bool,
    higher_priority: &[(bool, ProviderWarning)],
    warnings: &mut Vec<ProviderWarning>,
    build: B,
) -> Option<T>
where
    B: FnOnce() -> T,
{
    let outranked = higher_priority.iter().any(|&(present, _)| present);
    if enabled {
        for &(present, warning) in higher_priority {
            if present {
                warnings.push(warning);
            }
        }
    }
    (enabled && !outranked).then(build)
}
