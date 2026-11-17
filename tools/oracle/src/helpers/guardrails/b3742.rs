use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use serde::Deserialize;
use serde_json::Value;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod outcome;
mod poster;

const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";
const FIXED_NOW_SECONDS: u64 = 1_000_000;
const METADATA_KEY: &str = "metadata";
const RECORDS_KEY: &str = "standard_logging_guardrail_information";

type Data = BTreeMap<String, GuardrailValue>;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ScriptedReply {
    Reply {
        status: u16,
        body: Value,
        raw: Option<String>,
    },
    Unreachable,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    call_type: String,
    data: Data,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnforcementArgs {
    unreachable_fallback: UnreachableFallback,
    key_alias: Option<String>,
    replies: Vec<ScriptedReply>,
    repeat_last_reply: bool,
    invocations: Vec<Invocation>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = EnforcementArgs;

    fn run(input: &Self::Input) -> HelperResult {
        outcome::enforcement_result(input)
    }
}
