use std::collections::BTreeMap;

use betterlite_auth::auth_limit_inputs::key_limits::ModelRateLimitKey;
use betterlite_auth::auth_limit_inputs::metadata_limits::{
    MetadataAccessor, model_limit_from_metadata,
};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use indexmap::IndexMap;

use super::input::BatchInputAccountingInput;
use super::result::BatchInputRowUsage;
use crate::api::batch_limit_counters::rejections::PROJECT_OTPM_DESCRIPTOR_KEY;

// The running totals BatchFileUsage returns, each row's own usage kept under its body.model
#[derive(Debug, Default)]
pub(super) struct Totals {
    pub(super) total_tokens: i64,
    pub(super) request_count: i64,
    pub(super) output_tokens: i64,
    pub(super) per_model: IndexMap<String, BatchInputRowUsage>,
}

impl Totals {
    pub(super) fn add(&mut self, model: Option<&str>, total: i64, output: i64) {
        self.total_tokens = self.total_tokens.saturating_add(total);
        self.output_tokens = self.output_tokens.saturating_add(output);
        let Some(name) = model else {
            return;
        };
        let row = self.per_model.entry(name.to_owned()).or_default();
        row.total_tokens = row.total_tokens.saturating_add(total);
        row.output_tokens = row.output_tokens.saturating_add(output);
        row.request_count = row.request_count.saturating_add(1);
    }
}

// The project OTPM limits that tighten the no-max_tokens floor: this batch's descriptors first, then every one
pub(super) fn min_configured_otpm_limit(
    input: &BatchInputAccountingInput,
    caller: Option<&UserAPIKeyAuth>,
) -> Option<i64> {
    let descriptors = input
        .descriptors
        .iter()
        .filter(|descriptor| descriptor.key == PROJECT_OTPM_DESCRIPTOR_KEY)
        .filter_map(|descriptor| descriptor.rate_limit.as_ref()?.tokens_per_unit);
    let project = caller
        .and_then(|principal| {
            model_limit_from_metadata(
                principal,
                MetadataAccessor::Project,
                ModelRateLimitKey::Otpm,
            )
        })
        .filter(|limits| !limits.is_empty())
        .into_iter()
        .flat_map(BTreeMap::into_values);
    descriptors.chain(project).min()
}
