use crate::loader::{ConfigError, EnvironmentLookup};

use super::aws::build_aws;
use super::azure::resolve_azure;
use super::gcp::build_gcp;
use super::{Provider, ProviderWarning, RawSettings, ScalarBool, coerces_to_true};

pub(super) fn resolve_providers<L>(
    settings: &RawSettings,
    lookup: &L,
) -> Result<(Option<Provider>, Vec<ProviderWarning>), ConfigError>
where
    L: EnvironmentLookup,
{
    let mut warnings = Vec::new();
    let gcp = build_gcp(settings, lookup);

    let azure_ad_token = settings
        .azure_redis_ad_token
        .clone()
        .or_else(|| lookup.lookup("REDIS_AZURE_AD_TOKEN"));
    let azure_enabled = azure_ad_token.is_some_and(|value| value.eq_ignore_ascii_case("true"));
    let azure = resolve_provider(
        azure_enabled,
        &[(gcp.is_some(), ProviderWarning::GcpOutranksAzure)],
        &mut warnings,
        || resolve_azure(settings, lookup),
    );

    let aws = resolve_provider(
        aws_iam_enabled(settings, lookup),
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

fn aws_iam_enabled<L>(settings: &RawSettings, lookup: &L) -> bool
where
    L: EnvironmentLookup,
{
    let configured = settings.aws_iam_auth.as_ref();
    configured.map_or_else(
        || {
            lookup
                .lookup("REDIS_AWS_IAM_AUTH")
                .is_some_and(|value| coerces_to_true(&ScalarBool::Text(value)))
        },
        coerces_to_true,
    )
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
