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
    Fields(&'static [&'static str]),
    Object(&'static [(&'static str, Self)]),
    Objects(&'static Self),
    Tagged(&'static [(&'static str, Self)], &'static Self),
}

fn normalize(value: &Json, shape: &Shape) -> Json {
    match shape {
        Shape::Any => value.clone(),
        Shape::Fields(names) => normalize_fields(value, names),
        Shape::Object(fields) => normalize_object(value, fields),
        Shape::Objects(element) => normalize_list(value, element),
        Shape::Tagged(variants, unsupported) => normalize_tagged(value, variants, unsupported),
    }
}

// a fields object writes every declared key as given, which is an object whose keys have no shape
fn normalize_fields(value: &Json, names: &'static [&'static str]) -> Json {
    let fields: Vec<(&'static str, Shape)> = names.iter().map(|name| (*name, Shape::Any)).collect();
    normalize_object(value, &fields)
}

// an object writes every declared key in its shape's order and appends the keys the shape never names
fn normalize_object(value: &Json, fields: &[(&'static str, Shape)]) -> Json {
    let Json::Object(source) = value else {
        return value.clone();
    };
    let mut object = IndexMap::new();
    for (name, field) in fields {
        let normalized = normalize(source.get(*name).unwrap_or(&Json::Null), field);
        object.insert((*name).to_owned(), normalized);
    }
    append_extras(&mut object, source);
    Json::Object(object)
}

// the keys no shape names stay as the provider wrote them, after the declared ones
fn append_extras(object: &mut IndexMap<String, Json>, source: &IndexMap<String, Json>) {
    for (name, extra) in source {
        if !object.contains_key(name) {
            object.insert(name.clone(), extra.clone());
        }
    }
}

// a list is normalized element by element, and any other value stays as the provider wrote it
fn normalize_list(value: &Json, element: &Shape) -> Json {
    let Json::List(items) = value else {
        return value.clone();
    };
    Json::List(items.iter().map(|item| normalize(item, element)).collect())
}

// a tagged value takes its type's shape, item by item for a list, and the union's first member's shape
// for any type it doesn't name
fn normalize_tagged(
    value: &Json,
    variants: &'static [(&'static str, Shape)],
    unsupported: &'static Shape,
) -> Json {
    if let Json::List(items) = value {
        let normalized = items
            .iter()
            .map(|item| normalize_tagged(item, variants, unsupported));
        return Json::List(normalized.collect());
    }
    let Json::Object(source) = value else {
        return value.clone();
    };
    let Some(Json::Text(tag)) = source.get("type") else {
        return normalize(value, unsupported);
    };
    variants
        .iter()
        .find(|(name, _)| *name == tag.as_str())
        .map_or_else(
            || normalize(value, unsupported),
            |(_, shape)| normalize(value, shape),
        )
}

const ERROR: Shape = Shape::Fields(&["code", "message", "param"]);

const HYPERPARAMETERS: Shape =
    Shape::Fields(&["batch_size", "learning_rate_multiplier", "n_epochs"]);

const DPO_HYPERPARAMETERS: Shape =
    Shape::Fields(&["batch_size", "beta", "learning_rate_multiplier", "n_epochs"]);

const REINFORCEMENT_HYPERPARAMETERS: Shape = Shape::Fields(&[
    "batch_size",
    "compute_multiplier",
    "eval_interval",
    "eval_samples",
    "learning_rate_multiplier",
    "n_epochs",
    "reasoning_effort",
]);

const WANDB: Shape = Shape::Fields(&["project", "entity", "name", "tags"]);

const INTEGRATION: Shape = Shape::Object(&[("type", Shape::Any), ("wandb", WANDB)]);

// an Input's content block writes the keys its member declares
const CONTENT_TEXT: Shape = Shape::Fields(&["text", "type", "prompt_cache_breakpoint"]);
const CONTENT_OUTPUT_TEXT: Shape = Shape::Fields(&["text", "type"]);
const CONTENT_IMAGE: Shape = Shape::Fields(&["image_url", "type", "detail"]);
const AUDIO_DATA: Shape = Shape::Fields(&["data", "format"]);
const CONTENT_AUDIO: Shape = Shape::Object(&[("input_audio", AUDIO_DATA), ("type", Shape::Any)]);

// a content string, image, audio or grader list that names no block stays as the provider wrote it
const CONTENT: Shape = Shape::Tagged(
    &[
        ("input_text", CONTENT_TEXT),
        ("output_text", CONTENT_OUTPUT_TEXT),
        ("input_image", CONTENT_IMAGE),
        ("input_audio", CONTENT_AUDIO),
    ],
    &Shape::Any,
);

const GRADER_INPUT: Shape = Shape::Object(&[
    ("content", CONTENT),
    ("role", Shape::Any),
    ("type", Shape::Any),
]);

const STRING_CHECK_GRADER: Shape =
    Shape::Fields(&["input", "name", "operation", "reference", "type"]);

const TEXT_SIMILARITY_GRADER: Shape =
    Shape::Fields(&["evaluation_metric", "input", "name", "reference", "type"]);

const PYTHON_GRADER: Shape = Shape::Fields(&["name", "source", "type", "image_tag"]);

const SAMPLING_PARAMS: Shape = Shape::Fields(&[
    "max_completions_tokens",
    "reasoning_effort",
    "seed",
    "temperature",
    "top_p",
]);

const SCORE_MODEL_GRADER: Shape = Shape::Object(&[
    ("input", Shape::Objects(&GRADER_INPUT)),
    ("model", Shape::Any),
    ("name", Shape::Any),
    ("type", Shape::Any),
    ("range", Shape::Any),
    ("sampling_params", SAMPLING_PARAMS),
]);

// the union validates these four types and constructs its first member for every other type, since
// MultiGrader's graders field rejects each list before the union reaches it
const GRADER: Shape = Shape::Tagged(
    &[
        ("python", PYTHON_GRADER),
        ("score_model", SCORE_MODEL_GRADER),
        ("string_check", STRING_CHECK_GRADER),
        ("text_similarity", TEXT_SIMILARITY_GRADER),
    ],
    &STRING_CHECK_GRADER,
);

// a method object writes its four declared keys, and a reinforcement method its grader and hyperparameters
const METHOD: Shape = Shape::Object(&[
    ("type", Shape::Any),
    (
        "dpo",
        Shape::Object(&[("hyperparameters", DPO_HYPERPARAMETERS)]),
    ),
    (
        "reinforcement",
        Shape::Object(&[
            ("grader", GRADER),
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
