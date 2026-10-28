use crate::loader::{ConfigError, EnvironmentLookup};

use super::aws::{AwsSettings, build_aws};
use super::azure::{AzureSettings, resolve_azure};
use super::gcp::{GcpSettings, build_gcp};
use super::{Provider, ProviderWarning, RawSettings, ScalarBool, coerces_to_true};

pub(super) struct ProviderSelection {
    pub gcp: Option<GcpSettings>,
    pub azure: Option<AzureSettings>,
    pub aws: Option<AwsSettings>,
    pub warnings: Vec<ProviderWarning>,
}

pub(super) fn resolve_providers<L>(
    settings: &RawSettings,
    lookup: &L,
) -> Result<ProviderSelection, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut warnings = Vec::new();
    let gcp = build_gcp(settings, lookup);

    let azure_ad_enabled = settings
        .azure_redis_ad_token
        .clone()
        .or_else(|| lookup.lookup("REDIS_AZURE_AD_TOKEN"))
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    let azure = resolve_provider(
        azure_ad_enabled,
        &[(gcp.is_some(), ProviderWarning::GcpOutranksAzure)],
        &mut warnings,
        || resolve_azure(settings, lookup),
    );

    let aws_iam_enabled = settings.aws_iam_auth.as_ref().is_some_and(coerces_to_true)
        || lookup
            .lookup("REDIS_AWS_IAM_AUTH")
            .is_some_and(|value| coerces_to_true(&ScalarBool::Text(value)));
    let aws = resolve_provider(
        aws_iam_enabled,
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

    Ok(ProviderSelection {
        gcp,
        azure,
        aws,
        warnings,
    })
}

pub(super) const fn provider_precedence(
    gcp: Option<&GcpSettings>,
    azure: Option<&AzureSettings>,
    aws: Option<&AwsSettings>,
) -> Option<Provider> {
    if gcp.is_some() {
        Some(Provider::Gcp)
    } else if azure.is_some() {
        Some(Provider::Azure)
    } else if aws.is_some() {
        Some(Provider::Aws)
    } else {
        None
    }
}

fn resolve_provider<T>(
    enabled: bool,
    higher_priority: &[(bool, ProviderWarning)],
    warnings: &mut Vec<ProviderWarning>,
    build: impl FnOnce() -> T,
) -> Option<T> {
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
