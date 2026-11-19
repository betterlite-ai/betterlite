mod fakes;
mod report;
mod trace;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use std::collections::BTreeMap;
pub(crate) struct Op;
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) now: String,
    #[serde(default)]
    pub(crate) step_millis: u64,
    pub(crate) token: Option<String>,
    pub(crate) query_type: QueryType,
    #[serde(default = "default_true")]
    pub(crate) check_deprecated: bool,
    #[serde(default)]
    pub(crate) use_writer: bool,
    #[serde(default)]
    pub(crate) rows: Vec<Option<Step<BTreeMap<String, JsonData>>>>,
    #[serde(default)]
    pub(crate) deprecated: Vec<Option<Step<DeprecatedFields>>>,
    #[serde(default)]
    pub(crate) reconnects: Vec<ReconnectStep>,
}
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QueryType {
    FindUnique,
    FindAll,
}
#[derive(Debug, Deserialize)]
pub(crate) struct DeprecatedFields {
    pub(crate) active_token_id: String,
    // the deprecated row's revoke_at as whole seconds since the epoch, so both sides build the same instant
    pub(crate) revoke_at_seconds: i64,
}
#[derive(Debug, Deserialize)]
pub(crate) struct Fault {
    #[serde(rename = "type")]
    pub(crate) error_type: String,
    pub(crate) message: String,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step<T> {
    Data { data: T },
    Failure { error: Fault },
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum ReconnectStep {
    Outcome(bool),
    Missing,
}
const fn default_true() -> bool {
    true
}
impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        report::run(input)
    }
}
