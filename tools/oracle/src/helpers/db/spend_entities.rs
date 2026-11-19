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
    payload: Option<Value>,
    metadata: Option<Value>,
    saved_spend: Option<Value>,
    entity: Option<String>,
    response_cost: Option<Value>,
    team_id: Option<String>,
    org_id: Option<String>,
    project_id: Option<String>,
    agent_id: Option<String>,
    user_id: Option<String>,
    request_tags: Option<String>,
    attributed: Option<Vec<String>>,
    served_model_id: Option<String>,
    deployments: Option<BTreeMap<String, JsonObject>>,
    fail_at: Option<usize>,
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
