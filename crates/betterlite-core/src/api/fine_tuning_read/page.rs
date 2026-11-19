use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

// the SDK page is handed back undecoded: data and has_more stay, other keys are kept as extras
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FineTuningPage(pub Json);

pub fn decode_page(bytes: &[u8]) -> Result<FineTuningPage, serde_json::Error> {
    let value: Json = serde_json::from_slice(bytes)?;
    let normalized = normalize(&value, &PAGE);
    Ok(FineTuningPage(normalized))
}

// a declared key is always written with its shape, and an undeclared one stays as the provider wrote it
enum Shape {
    Any,
    Object(&'static [(&'static str, Shape)]),
    Objects(&'static Shape),
}

fn normalize(value: &Json, shape: &Shape) -> Json {
    match shape {
        Shape::Any => value.clone(),
        Shape::Object(fields) => normalize_object(value, fields),
        Shape::Objects(element) => normalize_list(value, element),
    }
}

// an object writes every declared key in its shape's order and appends the keys the shape never names
fn normalize_object(value: &Json, fields: &'static [(&'static str, Shape)]) -> Json {
    let Json::Object(source) = value else {
        return value.clone();
    };
    let mut object = IndexMap::new();
    for (name, field) in fields {
        let normalized = normalize(source.get(*name).unwrap_or(&Json::Null), field);
        object.insert((*name).to_owned(), normalized);
    }
    for (name, extra) in source {
        if !object.contains_key(name) {
            object.insert(name.clone(), extra.clone());
        }
    }
    Json::Object(object)
}

// a list is normalized element by element, and any other value stays as the provider wrote it
fn normalize_list(value: &Json, element: &Shape) -> Json {
    let Json::List(items) = value else {
        return value.clone();
    };
    Json::List(items.iter().map(|item| normalize(item, element)).collect())
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
