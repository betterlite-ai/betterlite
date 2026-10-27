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
mod config;
mod context;
mod core_ops;
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
mod openai;
mod openapi;
mod oracle;
mod paths;
mod policy;
mod prometheus;
mod prompts;
mod provider;
mod proxy;
mod rate_limit;
mod redact;
mod redis;
mod roi;
mod router;
mod sdk;
mod secret_managers;
mod secrets;
mod server;
mod streaming;
mod tokenizer;
mod vertex;
mod websearch;

use std::collections::BTreeMap;
use std::io::{self, Write as _};
use std::path::Path;

use crate::OracleError;
use crate::case::HelperCase;
use crate::compare;
use crate::process::sync_litellm;
use python::run_python;

pub(crate) use crate::compare::helper::{HelperError, HelperOutput};

type HelperResult = Result<serde_json::Value, Box<HelperError>>;

trait Helper {
    type Input: serde::de::DeserializeOwned;

    fn run(input: &Self::Input) -> HelperResult;

    fn decode_and_run(input: &serde_json::Value) -> HelperResult {
        let decoded = serde_json::from_value(input.clone()).map_err(|error| {
            Box::new(HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        Self::run(&decoded)
    }

    fn check(input: &serde_json::Value) -> Result<(), String> {
        serde_json::from_value::<Self::Input>(input.clone())
            .map(drop)
            .map_err(|error| error.to_string())
    }

    fn not_built(operation: &str) -> HelperResult {
        Err(Box::new(HelperError {
            class: "NotBuilt".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: format!("the {operation} helper is not built yet"),
        }))
    }
}

struct Group {
    invoke: fn(&str, &serde_json::Value) -> Option<HelperResult>,
    check: fn(&str, &serde_json::Value) -> Option<Result<(), String>>,
}

const GROUPS: [Group; 42] = [
    a2a::GROUP,
    anthropic::GROUP,
    auth::GROUP,
    azure::GROUP,
    bedrock::GROUP,
    cache::GROUP,
    config::GROUP,
    context::GROUP,
    core_ops::GROUP,
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
    openai::GROUP,
    openapi::GROUP,
    oracle::GROUP,
    paths::GROUP,
    policy::GROUP,
    prometheus::GROUP,
    prompts::GROUP,
    provider::GROUP,
    proxy::GROUP,
    rate_limit::GROUP,
    redact::GROUP,
    redis::GROUP,
    roi::GROUP,
    router::GROUP,
    sdk::GROUP,
    secret_managers::GROUP,
    secrets::GROUP,
    server::GROUP,
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

pub(crate) enum HelperRun {
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

pub(crate) fn run_case(case: &HelperCase, self_check: bool) -> Result<HelperRun, OracleError> {
    let python_input =
        serde_json::json!({"operation": case.call.operation, "input": case.call.input});
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    sync_litellm(&root)?;
    let left = match left_side(self_check) {
        HelperLeft::Native => invoke(&case.call),
        HelperLeft::Python => run_python(&root, &python_input, &case.env)?,
    };
    let right = run_python(&root, &python_input, &case.env)?;
    if let Some(difference) = compare::compare_helper(&left, &right, &case.compare.ignore) {
        let pointer = difference.pointer;
        writeln!(io::stderr().lock(), "FAIL {}: {pointer}", case.name)
            .map_err(OracleError::Output)?;
        return Ok(HelperRun::Difference);
    }
    writeln!(io::stdout().lock(), "PASS {}", case.name).map_err(OracleError::Output)?;
    Ok(HelperRun::Match)
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
            value: Some(value),
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

mod python;
mod tests;
