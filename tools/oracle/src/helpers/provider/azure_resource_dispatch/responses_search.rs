mod responses;
mod search;

use std::collections::BTreeMap;

use betterlite_openai::response_resources::request::InputItemsOrder;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(super) steps: Vec<Step>,
}

// One B544 or B546 call per step; each family keeps its own request and reply shapes
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    ResponsesUrl(UrlStep),
    ResponsesTransform(TransformStep),
    ResponsesResource(ResourceStep),
    ResponsesInputItems(InputItemsStep),
    SearchHeaders(HeadersStep),
    SearchUrl(SearchUrlStep),
    SearchRequest(SearchRequestStep),
    SearchReply(SearchReplyStep),
}

#[derive(Debug, Deserialize)]
pub(crate) struct UrlStep {
    pub(super) api_base: String,
    pub(super) api_version: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TransformStep {
    pub(super) model: String,
    pub(super) input: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResourceStep {
    pub(super) endpoint: String,
    pub(super) api_base: String,
    pub(super) response_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InputItemsStep {
    pub(super) api_base: String,
    pub(super) response_id: String,
    #[serde(default)]
    pub(super) after: Option<String>,
    #[serde(default)]
    pub(super) before: Option<String>,
    #[serde(default)]
    pub(super) include: Option<Vec<String>>,
    #[serde(default)]
    pub(super) limit: Option<i64>,
    #[serde(default)]
    pub(super) order: Option<InputItemsOrder>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HeadersStep {
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SearchUrlStep {
    pub(super) api_base: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SearchRequestStep {
    pub(super) model: String,
    pub(super) query: String,
    #[serde(default)]
    pub(super) connection_id: Option<String>,
    #[serde(default)]
    pub(super) max_results: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SearchReplyStep {
    pub(super) body: String,
}

pub(super) fn run(input: &Input) -> HelperResult {
    let mut answers = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        let (kind, value) = match step {
            Step::ResponsesUrl(_)
            | Step::ResponsesTransform(_)
            | Step::ResponsesResource(_)
            | Step::ResponsesInputItems(_) => responses::answer(step)?,
            Step::SearchHeaders(_)
            | Step::SearchUrl(_)
            | Step::SearchRequest(_)
            | Step::SearchReply(_) => search::answer(step)?,
        };
        answers.push(json!({"kind": kind, "value": value}));
    }
    Ok(json!({"steps": answers}))
}
