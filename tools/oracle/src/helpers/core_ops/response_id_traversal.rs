use std::collections::BTreeSet;

use betterlite_core::api::response_id_traversal::container::collect_container_ids;
use betterlite_core::api::response_id_traversal::metadata::AffinityMetadata;
use betterlite_core::api::response_id_traversal::reasoning::strip_encrypted_reasoning;
use betterlite_core::api::response_id_traversal::record::Record;
use betterlite_core::api::response_id_traversal::response_id::rewrite_response_id;
use betterlite_core::api::response_id_traversal::restore::restore_encrypted_item_ids;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{Helper, HelperError, HelperResult};
use logs::Logs;

mod logs;
mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ResponseRewrite {
        scenarios: Vec<RewriteScenario>,
    },
    #[serde(rename = "input_restore")]
    Restore {
        inputs: Vec<Record>,
    },
    ReasoningStrip {
        scenarios: Vec<StripScenario>,
    },
    ContainerCollection {
        responses: Vec<Option<Record>>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RewriteScenario {
    response: Record,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    litellm_metadata: Option<AffinityMetadata>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StripScenario {
    input: Record,
    #[serde(default)]
    strip_ids: Option<Vec<String>>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ResponseRewrite { scenarios } => {
                Ok(json!({ "scenarios": rewrite_scenarios(scenarios)? }))
            }
            Input::Restore { inputs } => Ok(json!({ "inputs": restore_inputs(inputs)? })),
            Input::ReasoningStrip { scenarios } => {
                Ok(json!({ "scenarios": strip_scenarios(scenarios)? }))
            }
            Input::ContainerCollection { responses } => {
                Ok(json!({ "responses": collected(responses)? }))
            }
        }
    }
}

fn to_value(record: &Record) -> Result<Value, Box<HelperError>> {
    serde_json::to_value(record).map_err(|error| {
        Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

fn observed<T, F>(call: F) -> (T, Option<&'static str>)
where
    F: FnOnce() -> T,
{
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let value = tracing::subscriber::with_default(subscriber, call);
    let diagnostic = if logs.take().is_empty() {
        None
    } else {
        Some("debug")
    };
    (value, diagnostic)
}

fn rewrite_scenarios(scenarios: &[RewriteScenario]) -> Result<Vec<Value>, Box<HelperError>> {
    scenarios
        .iter()
        .map(rewrite_scenario)
        .collect::<Result<Vec<_>, _>>()
}

fn rewrite_scenario(scenario: &RewriteScenario) -> Result<Value, Box<HelperError>> {
    let mut response = scenario.response.clone();
    let metadata = scenario.litellm_metadata.clone().unwrap_or_default();
    let (result, diagnostic) = observed(|| {
        rewrite_response_id(
            &mut response,
            scenario.custom_llm_provider.as_deref(),
            &metadata,
        )
    });
    if let Err(error) = result {
        return Err(Box::new(HelperError {
            class: "TraversalError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        }));
    }
    Ok(json!({
        "response": to_value(&response)?,
        "diagnostic": diagnostic,
    }))
}

fn restore_inputs(inputs: &[Record]) -> Result<Vec<Value>, Box<HelperError>> {
    let mut results = Vec::with_capacity(inputs.len());
    for input in inputs {
        let mut restored = input.clone();
        let ((), diagnostic) = observed(|| restore_encrypted_item_ids(&mut restored));
        results.push(json!({
            "input": to_value(&restored)?,
            "diagnostic": diagnostic,
        }));
    }
    Ok(results)
}

fn strip_scenarios(scenarios: &[StripScenario]) -> Result<Vec<Value>, Box<HelperError>> {
    let mut results = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        let mut stripped = scenario.input.clone();
        let selected: BTreeSet<&str> = scenario
            .strip_ids
            .as_ref()
            .map(|ids| ids.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let predicate = |item: &Record| {
            item.field("id")
                .and_then(Record::as_text)
                .is_some_and(|id| selected.contains(id))
        };
        let should_strip: Option<&dyn Fn(&Record) -> bool> =
            scenario.strip_ids.is_some().then_some(&predicate);
        let ((), diagnostic) = observed(|| strip_encrypted_reasoning(&mut stripped, should_strip));
        results.push(json!({
            "input": to_value(&stripped)?,
            "diagnostic": diagnostic,
        }));
    }
    Ok(results)
}

fn collected(responses: &[Option<Record>]) -> Result<Vec<Value>, Box<HelperError>> {
    let mut results = Vec::with_capacity(responses.len());
    for response in responses {
        let (ids, diagnostic) = observed(|| collect_container_ids(response.as_ref()));
        let mut sorted = ids;
        sorted.sort();
        let projected = match response {
            Some(record) => to_value(record)?,
            None => Value::Null,
        };
        results.push(json!({
            "ids": sorted,
            "response": projected,
            "diagnostic": diagnostic,
        }));
    }
    Ok(results)
}
