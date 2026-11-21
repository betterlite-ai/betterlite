use std::collections::BTreeMap;

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_rate_limit::limiter_records::RateLimitStatus;
use betterlite_rate_limit::provider_attribution::PriorityValue;
use betterlite_rate_limit::usage_resolution::Usage;
use serde::Deserialize;

mod answers;
mod fakes;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    // boxed so both variants carry a pointer, well within the variant_size_differences bound
    PostCall(Box<PostCallCase>),
    LogSuccess(Box<LogSuccessCase>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct PostCallCase {
    #[serde(default)]
    pub(crate) team_metadata: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub(crate) key_metadata: BTreeMap<String, JsonData>,
    #[serde(default)]
    pub(crate) statuses: Vec<RateLimitStatus>,
    pub(crate) hidden_params: bool,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, Json>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LogSuccessCase {
    #[serde(default)]
    pub(crate) auth_metadata: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) model_group: Option<String>,
    #[serde(default)]
    pub(crate) usage: Option<Usage>,
    pub(crate) rate_limit_type: TokenRateLimitType,
    pub(crate) window_size: i64,
    #[serde(default)]
    pub(crate) priority_reservation: Option<BTreeMap<String, PriorityValue>>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::PostCall(case) => answers::post_call_answer(case),
            Input::LogSuccess(case) => answers::log_success_answer(case),
        }
    }
}
