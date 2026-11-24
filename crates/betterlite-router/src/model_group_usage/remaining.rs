use std::time::SystemTime;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::io_token_reservations::limits::IoLimits;
use crate::model_group_info::group::ModelGroupInfo;
use crate::remaining_usage_headers::RemainingUsage;

use super::counters::UsageCounterCache;
use super::error::GroupUsageError;
use super::metadata::GroupMetadata;
use super::usage::{UsageSeries, group_counter_totals};

// ast-grep-ignore: no-long-names
use crate::remaining_usage_headers::build_io_token_rate_limit_headers;

// Router.get_remaining_model_group_usage: the io headers first, then the standard pair for whichever of tpm and rpm is set.
pub async fn group_remaining_usage<M, C>(
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
    let io_limits = match info.as_ref() {
        Some(group) => IoLimits {
            itpm: group.itpm,
            otpm: group.otpm,
        },
        None => IoLimits::default(),
    };
    if io_limits.itpm.is_some() || io_limits.otpm.is_some() {
        let (itpm, otpm) = group_counter_totals(UsageSeries::IoToken, model_list, cache, now)
            .await
            .map_err(GroupUsageError::Counter)?;
        // ast-grep-ignore: no-long-names
        headers.extend(build_io_token_rate_limit_headers(
            io_limits, itpm, otpm, now,
        ));
    }
    add_standard_headers(&mut headers, info.as_ref(), model_list, cache, now)
        .await
        .map_err(GroupUsageError::Counter)?;
    Ok(headers)
}

// The tpm/rpm pair: no counter read at all when neither limit is set, and a missing counter counts as no usage.
async fn add_standard_headers<C>(
    headers: &mut RemainingUsage,
    info: Option<&ModelGroupInfo>,
    model_list: Option<&[DeploymentSpec]>,
    cache: &C,
    now: SystemTime,
) -> Result<(), C::Error>
where
    C: UsageCounterCache,
{
    let tpm_limit = info.and_then(|group| group.tpm);
    let rpm_limit = info.and_then(|group| group.rpm);
    if tpm_limit.is_none() && rpm_limit.is_none() {
        return Ok(());
    }
    let (tpm, rpm) = group_counter_totals(UsageSeries::Standard, model_list, cache, now).await?;
    if let Some(limit) = tpm_limit {
        headers.insert(
            "x-ratelimit-remaining-tokens".to_owned(),
            limit.saturating_sub(tpm.unwrap_or(0)),
        );
        headers.insert("x-ratelimit-limit-tokens".to_owned(), limit);
    }
    if let Some(limit) = rpm_limit {
        headers.insert(
            "x-ratelimit-remaining-requests".to_owned(),
            limit.saturating_sub(rpm.unwrap_or(0)),
        );
        headers.insert("x-ratelimit-limit-requests".to_owned(), limit);
    }
    Ok(())
}
