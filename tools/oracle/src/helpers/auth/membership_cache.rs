use betterlite_auth::membership_load::cache::{CachedMembership, CachedPayload, classify_cached};
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

// The three payload shapes auth_checks.py can read back from the cache.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub(crate) enum CachedInput {
    Pending,
    Typed {
        membership: Box<LiteLLMTeamMembership>,
    },
    Stored {
        payload: Box<Json>,
    },
}

#[derive(Deserialize)]
pub(crate) struct Input {
    payloads: Vec<CachedInput>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = Vec::with_capacity(input.payloads.len());
        for payload in &input.payloads {
            results.push(report(payload)?);
        }
        Ok(json!({ "results": results }))
    }
}

fn report(payload: &CachedInput) -> HelperResult {
    let cached = match payload {
        CachedInput::Pending => CachedPayload::Pending,
        CachedInput::Typed { membership } => CachedPayload::Typed(membership.clone()),
        CachedInput::Stored { payload } => CachedPayload::Stored(payload.clone()),
    };
    match classify_cached(cached).or_value_error()? {
        CachedMembership::Membership(membership) => Ok(json!({
            "kind": "membership",
            "membership": serde_json::to_value(&*membership).or_value_error()?,
        })),
        CachedMembership::Absent => Ok(json!({ "kind": "absent" })),
        CachedMembership::Miss => Ok(json!({ "kind": "miss" })),
    }
}
