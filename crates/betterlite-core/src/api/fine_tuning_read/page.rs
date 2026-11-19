use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

// the SDK page the list call hands back without decoding: a cursor page with data and has_more,
// its other keys kept as extras, and each job built with every declared field present
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FineTuningPage(pub Value);

pub fn decode_page(bytes: &[u8]) -> Result<FineTuningPage, serde_json::Error> {
    let value = serde_json::from_slice(bytes)?;
    Ok(FineTuningPage(normalize(&value, &PAGE)))
}

// the keys a declared object always writes, with a shape per key; anything else stays as the
// provider wrote it, and a declared key whose value has another type also stays as written
enum Shape {
    Any,
    Object(&'static [(&'static str, Shape)]),
    Objects(&'static Shape),
}

fn normalize(value: &Value, shape: &Shape) -> Value {
    match shape {
        Shape::Any => value.clone(),
        Shape::Object(fields) => {
            let Value::Object(source) = value else {
                return value.clone();
            };
            let mut object = Map::new();
            for (name, field) in *fields {
                let normalized = source
                    .get(*name)
                    .map_or(Value::Null, |value| normalize(value, field));
                object.insert((*name).to_owned(), normalized);
            }
            for (name, extra) in source {
                if !fields.iter().any(|(declared, _)| declared == name) {
                    object.insert(name.clone(), extra.clone());
                }
            }
            Value::Object(object)
        }
        Shape::Objects(element) => {
            let Value::Array(items) = value else {
                return value.clone();
            };
            Value::Array(items.iter().map(|item| normalize(item, element)).collect())
        }
    }
}

const ERROR: Shape = Shape::Object(&[
    ("code", Shape::Any),
    ("message", Shape::Any),
    ("param", Shape::Any),
]);

const HYPERPARAMETERS: Shape = Shape::Object(&[
    ("batch_size", Shape::Any),
    ("learning_rate_multiplier", Shape::Any),
    ("n_epochs", Shape::Any),
]);

const DPO_HYPERPARAMETERS: Shape = Shape::Object(&[
    ("batch_size", Shape::Any),
    ("beta", Shape::Any),
    ("learning_rate_multiplier", Shape::Any),
    ("n_epochs", Shape::Any),
]);

const REINFORCEMENT_HYPERPARAMETERS: Shape = Shape::Object(&[
    ("batch_size", Shape::Any),
    ("compute_multiplier", Shape::Any),
    ("eval_interval", Shape::Any),
    ("eval_samples", Shape::Any),
    ("learning_rate_multiplier", Shape::Any),
    ("n_epochs", Shape::Any),
    ("reasoning_effort", Shape::Any),
]);

const WANDB: Shape = Shape::Object(&[
    ("project", Shape::Any),
    ("entity", Shape::Any),
    ("name", Shape::Any),
    ("tags", Shape::Any),
]);

const INTEGRATION: Shape = Shape::Object(&[("type", Shape::Any), ("wandb", WANDB)]);

// a method object writes its four declared keys and keeps hyperparameters as an extra
const METHOD: Shape = Shape::Object(&[
    ("type", Shape::Any),
    (
        "dpo",
        Shape::Object(&[("hyperparameters", DPO_HYPERPARAMETERS)]),
    ),
    (
        "reinforcement",
        Shape::Object(&[
            ("grader", Shape::Any),
            ("hyperparameters", REINFORCEMENT_HYPERPARAMETERS),
        ]),
    ),
    (
        "supervised",
        Shape::Object(&[("hyperparameters", HYPERPARAMETERS)]),
    ),
]);

const JOB: Shape = Shape::Object(&[
    ("id", Shape::Any),
    ("created_at", Shape::Any),
    ("error", ERROR),
    ("fine_tuned_model", Shape::Any),
    ("finished_at", Shape::Any),
    ("hyperparameters", HYPERPARAMETERS),
    ("model", Shape::Any),
    ("object", Shape::Any),
    ("organization_id", Shape::Any),
    ("result_files", Shape::Any),
    ("seed", Shape::Any),
    ("status", Shape::Any),
    ("trained_tokens", Shape::Any),
    ("training_file", Shape::Any),
    ("validation_file", Shape::Any),
    ("estimated_finish", Shape::Any),
    ("integrations", Shape::Objects(&INTEGRATION)),
    ("metadata", Shape::Any),
    ("method", METHOD),
]);

const PAGE: Shape = Shape::Object(&[("data", Shape::Objects(&JOB)), ("has_more", Shape::Any)]);
