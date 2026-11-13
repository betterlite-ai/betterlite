use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract as _;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_base::settings::GuardrailSettings;
use betterlite_guardrails::tool_policy::{RequestToolNames, ToolPolicyGuardrail, ToolPolicyLookup};
use indexmap::IndexMap;
use serde::Deserialize;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplyInput {
    initialized: bool,
    input_policies: BTreeMap<String, String>,
    output_policies: BTreeMap<String, String>,
    blocked_by_permission: BTreeMap<String, Vec<String>>,
    inputs: GuardrailApiInputs,
    input_type: GuardrailInputType,
    #[serde(default)]
    request_data: BTreeMap<String, GuardrailValue>,
    #[serde(default)]
    route_names: Option<RouteNames>,
}

// The names LiteLLM's extractor returns for this route, which the oracle case must match
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RouteNames {
    route: String,
    names: Vec<String>,
}

struct FakeStore {
    initialized: bool,
    input_policies: BTreeMap<String, String>,
    output_policies: BTreeMap<String, String>,
    blocked_by_permission: BTreeMap<String, Vec<String>>,
}

impl ToolPolicyLookup for FakeStore {
    fn is_initialized(&self) -> bool {
        self.initialized
    }

    fn effective_policies(
        &self,
        tool_names: &[String],
        key_permission_id: Option<&str>,
        team_permission_id: Option<&str>,
    ) -> BTreeMap<String, String> {
        let mut blocked: BTreeSet<&str> = BTreeSet::new();
        for permission_id in [key_permission_id, team_permission_id]
            .into_iter()
            .flatten()
        {
            let stripped = permission_id.trim();
            if stripped.is_empty() {
                continue;
            }
            if let Some(listed) = self.blocked_by_permission.get(stripped) {
                blocked.extend(listed.iter().map(String::as_str));
            }
        }
        tool_names
            .iter()
            .map(|name| {
                let policy = if blocked.contains(name.as_str()) {
                    "blocked"
                } else {
                    self.input_policies
                        .get(name)
                        .map_or("untrusted", String::as_str)
                };
                (name.clone(), policy.to_owned())
            })
            .collect()
    }

    fn output_policy(&self, tool_name: &str) -> String {
        self.output_policies
            .get(tool_name)
            .cloned()
            .unwrap_or_else(|| "untrusted".to_owned())
    }
}

struct FakeNames {
    route_names: Option<RouteNames>,
}

impl RequestToolNames for FakeNames {
    fn names(&self, route: &str, _request_data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
        let Some(expected) = &self.route_names else {
            return Vec::new();
        };
        if expected.route == route {
            return expected.names.clone();
        }
        Vec::new()
    }
}

#[derive(serde::Serialize)]
struct Blocked {
    status: u16,
    detail: IndexMap<String, GuardrailValue>,
}

#[derive(serde::Serialize)]
struct Output {
    blocked: Option<Blocked>,
    inputs: Option<GuardrailApiInputs>,
}

impl Helper for Op {
    type Input = ApplyInput;

    fn run(input: &Self::Input) -> HelperResult {
        let guardrail = ToolPolicyGuardrail::new(
            GuardrailSettings::default(),
            true,
            Arc::new(FakeStore {
                initialized: input.initialized,
                input_policies: input.input_policies.clone(),
                output_policies: input.output_policies.clone(),
                blocked_by_permission: input.blocked_by_permission.clone(),
            }),
            Arc::new(FakeNames {
                route_names: input.route_names.clone(),
            }),
        )
        .map_err(|error| failure("ValueError", error.to_string()))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure("RuntimeError", error.to_string()))?;
        let mut request_data = input.request_data.clone();
        let outcome = runtime.block_on(guardrail.apply_guardrail(
            input.inputs.clone(),
            &mut request_data,
            input.input_type,
            None,
        ));
        collect(outcome)
    }
}

fn collect(outcome: Result<GuardrailApiInputs, GuardrailFailure>) -> HelperResult {
    let output = match outcome {
        Ok(returned) => Output {
            blocked: None,
            inputs: Some(returned),
        },
        Err(GuardrailFailure::HttpObject { status, detail }) => Output {
            blocked: Some(Blocked { status, detail }),
            inputs: None,
        },
        Err(other) => return Err(failure("RuntimeError", other.to_string())),
    };
    serde_json::to_value(output).map_err(|error| failure("ValueError", error.to_string()))
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
