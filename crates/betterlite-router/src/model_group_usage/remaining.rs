use std::time::SystemTime;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::io_token_reservations::limits::IoLimits;
use crate::remaining_usage_headers::{RemainingUsage, build_io_token_rate_limit_headers};

use super::counters::UsageCounterCache;
use super::error::GroupUsageError;
use super::metadata::GroupMetadata;
use super::usage::{get_model_group_io_token_usage, get_model_group_usage};

// Router.get_remaining_model_group_usage: the io headers first, then the standard pair for whichever of tpm and rpm is set.
pub async fn get_remaining_model_group_usage<M, C>(
    model_group: &str,
    model_list: Option<&[DeploymentSpec]>,
    metadata: &M,
    cache: &C,
    now: SystemTime,
) -> Result<RemainingUsage, GroupUsageError<M::Error, C::Error>>
where
    M: GroupMetadata,
    C: UsageCounterCache,
{
    let info = metadata
        .cached_group_info(model_group)
        .await
        .map_err(GroupUsageError::Metadata)?;
    let mut headers = RemainingUsage::new();
    let io_limits = info.as_ref().map_or(IoLimits::default(), |info| IoLimits {
        itpm: info.itpm,
        otpm: info.otpm,
    });
    if io_limits.itpm.is_some() || io_limits.otpm.is_some() {
        let usage = get_model_group_io_token_usage(model_list, cache, now)
            .await
            .map_err(GroupUsageError::Counter)?;
        headers.extend(build_io_token_rate_limit_headers(
            io_limits, usage.itpm, usage.otpm, now,
        ));
    }
    let tpm_limit = info.as_ref().and_then(|info| info.tpm);
    let rpm_limit = info.as_ref().and_then(|info| info.rpm);
    if tpm_limit.is_none() && rpm_limit.is_none() {
        return Ok(headers);
    }
    let usage = get_model_group_usage(model_list, cache, now)
        .await
        .map_err(GroupUsageError::Counter)?;
    if let Some(limit) = tpm_limit {
        headers.insert(
            "x-ratelimit-remaining-tokens".to_owned(),
            limit.saturating_sub(usage.tpm.unwrap_or(0)),
        );
        headers.insert("x-ratelimit-limit-tokens".to_owned(), limit);
    }
    if let Some(limit) = rpm_limit {
        headers.insert(
            "x-ratelimit-remaining-requests".to_owned(),
            limit.saturating_sub(usage.rpm.unwrap_or(0)),
        );
        headers.insert("x-ratelimit-limit-requests".to_owned(), limit);
    }
    Ok(headers)
}
