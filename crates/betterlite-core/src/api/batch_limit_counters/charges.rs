use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use super::ports::BatchCounterLimiter;
use super::records::BatchFileUsage;
use crate::api::batch_limit_descriptors::descriptors::{
    BatchDescriptorIncrements, BatchLimitDescriptor,
};
use crate::api::request_destination::RequestData;

pub(super) fn charged_counters<Limiter>(
    limiter: &Limiter,
    auth: &UserAPIKeyAuth,
    data: &RequestData,
    batch_usage: &BatchFileUsage,
    descriptors: Option<&[BatchLimitDescriptor]>,
) -> Result<(Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>), Limiter::Error>
where
    Limiter: BatchCounterLimiter,
{
    let mut charged = descriptors.map_or_else(
        || limiter.base_descriptors(auth, data),
        |given| Ok(given.to_vec()),
    )?;
    let mut increments: Vec<BatchDescriptorIncrements> = charged
        .iter()
        .map(|_descriptor| BatchDescriptorIncrements {
            requests: batch_usage.request_count,
            tokens: batch_usage.total_tokens,
        })
        .collect();
    let (project, project_increments) =
        limiter.project_io_descriptors(auth, &batch_usage.project_usage());
    charged.extend(project);
    increments.extend(project_increments);
    Ok((charged, increments))
}
