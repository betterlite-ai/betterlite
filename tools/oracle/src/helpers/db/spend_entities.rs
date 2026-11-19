use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::Value;

mod enqueue;
mod turn;

use self::enqueue::enqueue_value;
use self::turn::turn_value;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

// One oracle call: a turn through the rollup builder, or one entity enqueue through the writer.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    kind: String,
    #[serde(default)]
    payload: Option<Value>,
    #[serde(default)]
    metadata: Option<Value>,
    #[serde(default)]
    saved_spend: Option<Value>,
    #[serde(default)]
    entity: Option<String>,
    #[serde(default)]
    response_cost: Option<Value>,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    org_id: Option<String>,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    request_tags: Option<String>,
    #[serde(default)]
    attributed: Option<Vec<String>>,
    #[serde(default)]
    served_model_id: Option<String>,
    #[serde(default)]
    deployments: Option<BTreeMap<String, JsonObject>>,
    #[serde(default)]
    fail_at: Option<usize>,
    #[serde(default)]
    database: Option<bool>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            results.push(match step.kind.as_str() {
                "turn" => turn_value(step)?,
                "enqueue" => enqueue_value(step)?,
                other => {
                    return Err(failure(format!(
                        "unknown spend_entities step kind: {other}"
                    )));
                }
            });
        }
        serde_json::to_value(results).map_err(failure)
    }
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "SpendEntities".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
