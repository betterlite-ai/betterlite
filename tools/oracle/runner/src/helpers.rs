use std::collections::BTreeMap;
use std::io::{self, Write as _};
use std::iter;
use std::path::Path;
use std::process::Command;

use betterlite_oracle_helper_api::group::Group;

use crate::case::helper::HelperCase;
use crate::compare;
use crate::compare::helper::HelperOutput;
use crate::error::HelperRunError;
use answers::Answers;
use betterlite_oracle_helper_api::error::HelperError;
use python::{process_environment, python_stdout};

// tools/oracle, where the cases, the fuzz corpus, the fixtures and the Python helpers live
pub const ORACLE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(try_from = "BTreeMap<String, serde_json::Value>")]
pub struct HelperCall {
    pub operation: String,
    pub input: serde_json::Value,
}

impl TryFrom<BTreeMap<String, serde_json::Value>> for HelperCall {
    type Error = String;

    fn try_from(call: BTreeMap<String, serde_json::Value>) -> Result<Self, Self::Error> {
        let mut entries = call.into_iter();
        match (entries.next(), entries.next()) {
            (Some((operation, input)), None) => Ok(Self { operation, input }),
            _ => Err("a helper call names exactly one operation".to_owned()),
        }
    }
}

impl HelperCall {
    // a group crate sees only its own operations, so the oracle checks every call against all groups when it loads a case
    pub fn check(&self, groups: &[Group]) -> Result<(), String> {
        groups
            .iter()
            .find_map(|group| (group.check)(&self.operation, &self.input))
            .unwrap_or_else(|| {
                Err(format!(
                    "unknown variant `{}`, expected a registered helper operation",
                    self.operation
                ))
            })
    }
}

pub enum HelperRun {
    Match,
    Difference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperLeft {
    Native,
    Python,
}

#[must_use]
pub const fn left_side(self_check: bool) -> HelperLeft {
    if self_check {
        HelperLeft::Python
    } else {
        HelperLeft::Native
    }
}

pub fn run_case(
    groups: &[Group],
    case: &HelperCase,
    self_check: bool,
) -> Result<HelperRun, HelperRunError> {
    let total = case.calls.len().saturating_add(1);
    for (index, call) in iter::once(&case.call).chain(case.calls.iter()).enumerate() {
        let label = if total > 1 {
            format!("{} call {index}", case.name)
        } else {
            case.name.clone()
        };
        let (left, right) = run_pair(groups, call, &case.env, self_check)?;
        let Some(difference) = compare::compare_helper(&left, &right, &case.compare.ignore) else {
            continue;
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
            "FAIL {label}: {pointer} ({})\n     {left_name}: {left_value}\n     {right_name}: {right_value}",
            difference.message
        )
        .map_err(HelperRunError::Output)?;
        return Ok(HelperRun::Difference);
    }
    writeln!(io::stdout().lock(), "PASS {}", case.name).map_err(HelperRunError::Output)?;
    Ok(HelperRun::Match)
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

pub fn run_pair(
    groups: &[Group],
    call: &HelperCall,
    env: &BTreeMap<String, String>,
    self_check: bool,
) -> Result<(HelperOutput, HelperOutput), HelperRunError> {
    let root = Path::new(ORACLE_DIR).join("../..");
    Ok(match left_side(self_check) {
        HelperLeft::Native => {
            let answers = Answers::from_env(&root)?;
            (invoke(groups, call), answers.answer(&root, call, env)?)
        }
        HelperLeft::Python => {
            sync_litellm(&root)?;
            let process_env = process_environment();
            let [first, second] = [(), ()].map(|()| {
                let stdout_output = python_stdout(&root, call, env, &process_env)?;
                serde_json::from_slice::<HelperOutput>(&stdout_output)
                    .map_err(HelperRunError::HelperJson)
            });
            (first?, second?)
        }
    })
}

#[must_use]
pub fn invoke(groups: &[Group], call: &HelperCall) -> HelperOutput {
    let result = groups
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

pub fn sync_litellm(root: &Path) -> Result<(), HelperRunError> {
    let project = root.join("tools/oracle/litellm");
    let status = Command::new("uv")
        .args(["sync", "--locked"])
        .current_dir(project)
        .status()
        .map_err(HelperRunError::ProcessSync)?;
    if status.success() {
        Ok(())
    } else {
        Err(HelperRunError::ProcessSyncExited(status))
    }
}

mod answers;
mod python;
mod tests;
