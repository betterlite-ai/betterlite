use std::collections::BTreeMap;

use betterlite_models::ptu_pricing::spillover::azure_spillover;
use betterlite_models::ptu_pricing::{
    PtuModelInfo, declares_ptu, ptu_config_error, ptu_identity_error, ptu_terms,
};
use betterlite_models::tiered_pricing::Scalar;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Terms,
    ConfigError,
    Identity,
    Declares,
    Spillover,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default)]
    model_info: BTreeMap<String, Scalar>,
    model_name: Option<String>,
    declared_id: Option<String>,
    #[serde(default)]
    taken: bool,
    current_id: Option<String>,
    response_headers: Option<BTreeMap<String, String>>,
    additional_headers: Option<BTreeMap<String, String>>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let model_info = PtuModelInfo::from(&input.model_info);
        let value = match input.operation {
            Operation::Terms => match ptu_terms(&model_info) {
                Some(terms) => json!({
                    "team_id": terms.team_id,
                    "ptu_count": terms.ptu_count,
                    "cost_per_ptu_per_hour": terms.cost_per_ptu_per_hour.normalize().to_string(),
                    "effective_from": instant_text(terms.effective_from),
                    "effective_to": terms.effective_to.map(instant_text),
                }),
                None => Value::Null,
            },
            Operation::ConfigError => ptu_config_error(&model_info, input.model_name.as_deref())
                .map_or(Value::Null, Value::String),
            Operation::Identity => ptu_identity_error(
                input.declared_id.as_deref(),
                input.taken,
                input.current_id.as_deref(),
                input.model_name.as_deref(),
            )
            .map_or(Value::Null, Value::String),
            Operation::Declares => Value::Bool(declares_ptu(&model_info)),
            Operation::Spillover => azure_spillover(
                input.response_headers.as_ref(),
                input.additional_headers.as_ref(),
            )
            .map_or(
                Value::Null,
                |found| json!({"from_deployment": found.from_deployment}),
            ),
        };
        Ok(value)
    }
}

// Python's isoformat writes the fraction only when it is non-zero, in six digits.
fn instant_text(value: DateTime<Utc>) -> String {
    if value.timestamp_subsec_micros() == 0 {
        return value.format("%Y-%m-%dT%H:%M:%S+00:00").to_string();
    }
    value.format("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string()
}
