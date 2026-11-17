use betterlite_core::api::vector_stores::answer::decorate_answer;
use betterlite_core::api::vector_stores::streaming::decorate_chunk;
use betterlite_core::api::vector_stores::{PROVIDER_FIELDS, SEARCH_FAILURES, SEARCH_RESULTS};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::json;

const CHOICES: &str = "choices";
const MESSAGE: &str = "message";
const DELTA: &str = "delta";
const OUTPUT: &str = "output";
const ID: &str = "id";
const OBJECT: &str = "object";

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Success {
        request_data: IndexMap<String, Json>,
        response: Json,
    },
    Streaming {
        request_data: IndexMap<String, Json>,
        response: Json,
    },
}

#[derive(Debug, Serialize)]
struct ChoiceProjection<'data> {
    message: Option<&'data Json>,
    delta: Option<&'data Json>,
}

// LiteLLM's pydantic objects carry constructor defaults a decoded record never had, so both sides compare only the fields the hooks change.
#[derive(Debug, Serialize)]
struct Projection<'data> {
    id: Option<&'data Json>,
    object: Option<&'data Json>,
    output: Option<&'data Json>,
    vector_store_search_failures: Option<&'data Json>,
    search_results: Option<&'data Json>,
    choices: Vec<ChoiceProjection<'data>>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Success {
                request_data,
                response,
            } => {
                let mut answer = response.clone();
                let decoration = decorate_answer(request_data, &mut answer);
                let returned = decoration.returns_answer.then(|| project(&answer));
                Ok(json!({
                    "returned": returned,
                    "diagnostics": decoration.diagnostics,
                }))
            }
            Input::Streaming {
                request_data,
                response,
            } => {
                let mut chunk = response.clone();
                let diagnostics = decorate_chunk(request_data, &mut chunk);
                Ok(json!({
                    "returned": project(&chunk),
                    "diagnostics": diagnostics,
                }))
            }
        }
    }
}

fn project(record: &Json) -> Projection<'_> {
    let Json::Object(entry) = record else {
        return Projection {
            id: None,
            object: None,
            output: None,
            vector_store_search_failures: None,
            search_results: None,
            choices: Vec::new(),
        };
    };
    Projection {
        id: entry.get(ID),
        object: entry.get(OBJECT),
        output: entry.get(OUTPUT),
        vector_store_search_failures: entry.get(SEARCH_FAILURES),
        search_results: entry.get(SEARCH_RESULTS),
        choices: projected_choices(entry),
    }
}

fn projected_choices(record: &IndexMap<String, Json>) -> Vec<ChoiceProjection<'_>> {
    let Some(Json::List(choices)) = record.get(CHOICES) else {
        return Vec::new();
    };
    choices.iter().map(project_choice).collect()
}

fn project_choice(choice: &Json) -> ChoiceProjection<'_> {
    let Json::Object(entry) = choice else {
        return ChoiceProjection {
            message: None,
            delta: None,
        };
    };
    ChoiceProjection {
        message: provider_fields(entry, MESSAGE),
        delta: provider_fields(entry, DELTA),
    }
}

fn provider_fields<'data>(
    choice: &'data IndexMap<String, Json>,
    member: &str,
) -> Option<&'data Json> {
    let Some(Json::Object(target)) = choice.get(member) else {
        return None;
    };
    target.get(PROVIDER_FIELDS)
}
