macro_rules! helper_group {
    ($($operation:literal => $module:ident),* $(,)?) => {
        $(mod $module;)*

        pub(super) const GROUP: super::Group = super::Group { invoke, check };

        fn invoke(
            operation: &str,
            input: &serde_json::Value,
        ) -> Option<super::HelperResult> {
            match operation {
                $($operation => Some(<$module::Op as super::Helper>::decode_and_run(input)),)*
                _ => None,
            }
        }

        fn check(operation: &str, input: &serde_json::Value) -> Option<Result<(), String>> {
            match operation {
                $($operation => Some(<$module::Op as super::Helper>::check(input)),)*
                _ => None,
            }
        }
    };
}

mod a2a;
mod anthropic;
mod auth;
mod azure;
mod bedrock;
mod cache;
mod cli;
mod config;
mod context;
mod cost;
mod db;
mod error;
mod foundry;
mod guardrails;
mod harness;
mod http;
mod lens;
mod logging;
mod mcp;
mod models;
mod ocr;
mod openai;
mod openapi;
mod oracle;
mod paths;
mod policy;
mod prometheus;
mod prompts;
mod provider;
mod proxy;
mod pyhost;
mod rate_limit;
mod redis;
mod reporting;
mod roi;
mod router;
mod sdk;
mod secret_managers;
mod secrets;
mod streaming;
mod tokenizer;
mod vertex;
mod websearch;

use std::collections::BTreeMap;
use std::io::{self, Write as _};
use std::path::Path;

use betterlite_oracle_helper_api::group::Group;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use crate::OracleError;
use crate::case::HelperCase;
use crate::compare;
use crate::process::sync_litellm;
use answers::Answers;
use python::python_stdout;

pub(crate) use crate::compare::helper::{HelperError, HelperOutput};
pub(crate) use betterlite_oracle_helper_api::error::OrHelperError;

const GROUPS: [Group; 46] = [
    a2a::GROUP,
    anthropic::GROUP,
    auth::GROUP,
    azure::GROUP,
    bedrock::GROUP,
    cache::GROUP,
    cli::GROUP,
    config::GROUP,
    context::GROUP,
    betterlite_oracle_core_ops::GROUP,
    cost::GROUP,
    db::GROUP,
    error::GROUP,
    foundry::GROUP,
    guardrails::GROUP,
    harness::GROUP,
    http::GROUP,
    lens::GROUP,
    logging::GROUP,
    mcp::GROUP,
    models::GROUP,
    ocr::GROUP,
    openai::GROUP,
    openapi::GROUP,
    oracle::GROUP,
    paths::GROUP,
    policy::GROUP,
    prometheus::GROUP,
    prompts::GROUP,
    provider::GROUP,
    proxy::GROUP,
    pyhost::GROUP,
    rate_limit::GROUP,
    betterlite_oracle_redact::GROUP,
    redis::GROUP,
    reporting::GROUP,
    roi::GROUP,
    router::GROUP,
    sdk::GROUP,
    secret_managers::GROUP,
    secrets::GROUP,
    betterlite_oracle_server::GROUP,
    streaming::GROUP,
    tokenizer::GROUP,
    vertex::GROUP,
    websearch::GROUP,
];

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(
    try_from = "BTreeMap<String, serde_json::Value>",
    into = "BTreeMap<String, serde_json::Value>"
)]
pub(crate) struct HelperCall {
    pub(crate) operation: String,
    pub(crate) input: serde_json::Value,
}

impl TryFrom<BTreeMap<String, serde_json::Value>> for HelperCall {
    type Error = String;

    fn try_from(call: BTreeMap<String, serde_json::Value>) -> Result<Self, Self::Error> {
        let mut entries = call.into_iter();
        match (entries.next(), entries.next()) {
            (Some((operation, input)), None) => {
                let checked = GROUPS
                    .iter()
                    .find_map(|group| (group.check)(&operation, &input));
                checked.unwrap_or_else(|| {
                    Err(format!(
                        "unknown variant `{operation}`, expected a registered helper operation"
                    ))
                })?;
                Ok(Self { operation, input })
            }
            _ => Err("a helper call names exactly one operation".to_owned()),
        }
    }
}

impl From<HelperCall> for BTreeMap<String, serde_json::Value> {
    fn from(call: HelperCall) -> Self {
        Self::from([(call.operation, call.input)])
    }
}

pub enum HelperRun {
    Match,
    Difference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HelperLeft {
    Native,
    Python,
}

pub(crate) const fn left_side(self_check: bool) -> HelperLeft {
    if self_check {
        HelperLeft::Python
    } else {
        HelperLeft::Native
    }
}

pub fn run_case(case: &HelperCase, self_check: bool) -> Result<HelperRun, OracleError> {
    let (left, right) = run_pair(&case.call, &case.env, self_check)?;
    let Some(difference) = compare::compare_helper(&left, &right, &case.compare.ignore) else {
        writeln!(io::stdout().lock(), "PASS {}", case.name).map_err(OracleError::Output)?;
        return Ok(HelperRun::Match);
    };
    let pointer = difference.pointer;
    let (left_name, right_name) = match left_side(self_check) {
        HelperLeft::Native => ("betterlite", "litellm"),
        HelperLeft::Python => ("first litellm run", "second litellm run"),
    };
    let left_value = value_at(&left, &pointer);
    let right_value = value_at(&right, &pointer);
    writeln!(
        io::stderr().lock(),
        "FAIL {}: {pointer} ({})\n     {left_name}: {left_value}\n     {right_name}: {right_value}",
        case.name,
        difference.message
    )
    .map_err(OracleError::Output)?;
    Ok(HelperRun::Difference)
}

fn value_at(output: &HelperOutput, pointer: &str) -> String {
    serde_json::to_value(output)
        .ok()
        .and_then(|value| value.pointer(pointer).map(ToString::to_string))
        .map_or_else(
            || "absent".to_owned(),
            |text| text.chars().take(400).collect(),
        )
}

pub(crate) fn run_pair(
    call: &HelperCall,
    env: &BTreeMap<String, String>,
    self_check: bool,
) -> Result<(HelperOutput, HelperOutput), OracleError> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    Ok(match left_side(self_check) {
        HelperLeft::Native => {
            let answers = Answers::from_env(&root)?;
            (invoke(call), answers.answer(&root, call, env)?)
        }
        HelperLeft::Python => {
            sync_litellm(&root)?;
            let [first, second] = [(), ()].map(|()| {
                let stdout_output = python_stdout(&root, call, env)?;
                serde_json::from_slice::<HelperOutput>(&stdout_output)
                    .map_err(OracleError::HelperJson)
            });
            (first?, second?)
        }
    })
}

pub(crate) fn invoke(call: &HelperCall) -> HelperOutput {
    let result = GROUPS
        .iter()
        .find_map(|group| (group.invoke)(&call.operation, &call.input))
        .unwrap_or_else(|| {
            let message = format!("no helper operation is named {}", call.operation);
            Err(Box::new(HelperError {
                class: "UnknownOperation".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message,
            }))
        });
    match result {
        Ok(value) => HelperOutput {
            ok: true,
            // the Python side reads a null value as absent, so a Rust null goes out the same way
            value: (!value.is_null()).then_some(value),
            error: None,
            warnings: Vec::new(),
            effects: Vec::new(),
        },
        Err(failure) => {
            let mut error = *failure;
            error.error_type.get_or_insert(None);
            error.param.get_or_insert(None);
            error.code.get_or_insert(None);
            HelperOutput {
                ok: false,
                value: None,
                error: Some(error),
                warnings: Vec::new(),
                effects: Vec::new(),
            }
        }
    }
}

mod answers;
mod python;
mod tests;
