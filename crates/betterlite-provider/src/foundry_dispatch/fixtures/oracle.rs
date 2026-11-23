mod dispatch;
mod values;

use std::collections::BTreeMap;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};

// The input one foundry_provider oracle case sends; one observation comes back for each entry
#[derive(Deserialize)]
pub struct Case {
    inputs: Vec<Input>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Input {
    Dispatch(DispatchCase),
    Embedding(EmbeddingCase),
    Rewrite(RewriteCase),
}

#[derive(Deserialize)]
struct GlobalsCase {
    api_base: Option<String>,
    api_key: Option<String>,
    #[serde(default)]
    headers: Option<BTreeMap<String, String>>,
}

#[derive(Deserialize)]
struct DispatchCase {
    model: String,
    messages: Vec<Object>,
    api_base: Option<String>,
    api_key: Option<String>,
    #[serde(default)]
    headers: Option<BTreeMap<String, String>>,
    globals: GlobalsCase,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    status: Option<u16>,
}

#[derive(Deserialize)]
struct EmbeddingCase {
    api_base: Option<String>,
    api_key: Option<String>,
    globals: GlobalsCase,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct RewriteCase {
    base: String,
}

#[derive(Serialize)]
pub struct PostCall {
    error: bool,
    headers: bool,
}

// What both helpers can see at the seam main.py calls: the branch, the resolved credentials and the log
#[derive(Serialize)]
pub struct Observation {
    branch: Option<&'static str>,
    base: Option<String>,
    key: Option<String>,
    headers: Option<BTreeMap<String, String>>,
    messages: JsonValue,
    post_calls: Vec<PostCall>,
    errors: bool,
}

#[cfg_attr(test, mutants::skip)]
pub fn execute(case: &Case) -> Vec<Observation> {
    case.inputs.iter().map(observe).collect()
}

#[cfg_attr(test, mutants::skip)]
fn observe(input: &Input) -> Observation {
    match input {
        Input::Dispatch(case) => dispatch::dispatch_case(case),
        Input::Embedding(case) => dispatch::embedding_case(case),
        Input::Rewrite(case) => dispatch::rewrite_case(case),
    }
}
