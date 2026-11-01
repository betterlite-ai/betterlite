use crate::analysis::{AnalysisError, Candidate};
use serde::Serialize;
use std::collections::BTreeMap;

const TASK_INSTRUCTIONS: &str = "Group these observations into patterns by check and cause. Each execution_id is a compact reference to a whole group; copy those references exactly. Merge only the same check, kind and cause. Keep recovered errors separate from unresolved failures. Preserve every distinct supported problem and useful positive pattern. Each input reference must appear exactly once. Merge paraphrases of the same behavior, including an individual example and a broader pattern covering that example. Do not make separate groups just because different runs or numbers were involved. Return candidates with the union of their input references. Preserve their issue/pattern kind. Do not reinterpret evidence or create new facts. A candidate is a hypothesis to investigate.";

#[derive(Serialize)]
struct Prompt<'prompt> {
    task: &'prompt str,
    response_schema: Schema,
    candidates: Vec<Candidate>,
}

#[derive(Clone, Serialize)]
struct Schema {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    kind: Option<&'static str>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    properties: BTreeMap<&'static str, Self>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    required: Vec<&'static str>,
    #[serde(
        rename = "additionalProperties",
        skip_serializing_if = "Option::is_none"
    )]
    additional_properties: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    items: Option<Box<Self>>,
    #[serde(rename = "maxLength", skip_serializing_if = "Option::is_none")]
    max_length: Option<usize>,
    #[serde(rename = "enum", skip_serializing_if = "Vec::is_empty")]
    choices: Vec<&'static str>,
}

pub(super) fn cluster_request(candidates: Vec<Candidate>) -> Result<String, AnalysisError> {
    let prompt = Prompt {
        task: TASK_INSTRUCTIONS,
        response_schema: candidate_list_schema(),
        candidates,
    };
    Ok(serde_json::to_string(&prompt)?)
}

fn candidate_list_schema() -> Schema {
    let items = Schema {
        kind: Some("array"),
        properties: BTreeMap::new(),
        required: Vec::new(),
        additional_properties: None,
        items: Some(Box::new(candidate_schema())),
        max_length: None,
        choices: Vec::new(),
    };
    object_schema(BTreeMap::from([("candidates", items)]), vec!["candidates"])
}

fn candidate_schema() -> Schema {
    let required = vec!["check_id", "title", "hypothesis", "execution_ids"];
    object_schema(candidate_properties(), required)
}

fn object_schema(
    properties: BTreeMap<&'static str, Schema>,
    required: Vec<&'static str>,
) -> Schema {
    let base = Schema {
        kind: Some("object"),
        properties,
        required,
        additional_properties: None,
        items: None,
        max_length: None,
        choices: Vec::new(),
    };
    Schema {
        additional_properties: Some(false),
        ..base
    }
}

fn candidate_properties() -> BTreeMap<&'static str, Schema> {
    let string_base = Schema {
        kind: Some("string"),
        properties: BTreeMap::new(),
        required: Vec::new(),
        additional_properties: None,
        items: None,
        max_length: None,
        choices: Vec::new(),
    };
    let kind_field = Schema {
        choices: vec!["issue", "pattern"],
        ..string_base.clone()
    };
    let title_field = Schema {
        max_length: Some(160),
        ..string_base.clone()
    };
    let hypothesis_field = Schema {
        max_length: Some(2000),
        ..string_base.clone()
    };
    let execution_ids_field = Schema {
        kind: Some("array"),
        items: Some(Box::new(string_base.clone())),
        ..string_base.clone()
    };
    BTreeMap::from([
        ("check_id", string_base),
        ("kind", kind_field),
        ("title", title_field),
        ("hypothesis", hypothesis_field),
        ("execution_ids", execution_ids_field),
    ])
}
