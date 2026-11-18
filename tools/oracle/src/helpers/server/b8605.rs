use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::stream_cleanup::marks::mark_request_disconnected;
use betterlite_server::stream_cleanup::model::{client_named_model, later_chunk_model};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    RecordDisconnect { requests: Vec<JsonObject> },
    ModelChecks { checks: Vec<ModelCheck> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelCheck {
    chunks: Vec<JsonObject>,
    assembled: Option<Json>,
    request_data: JsonObject,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answer = match input {
            Input::RecordDisconnect { requests } => {
                Json::List(requests.iter().map(marked_request).collect())
            }
            Input::ModelChecks { checks } => Json::List(checks.iter().map(model_answer).collect()),
        };
        Ok(serde_json::to_value(answer).unwrap_or_default())
    }
}

fn marked_request(request_data: &JsonObject) -> Json {
    let mut marked = request_data.clone();
    mark_request_disconnected(&mut marked);
    Json::Object(marked)
}

fn model_answer(check: &ModelCheck) -> Json {
    let mut answer = JsonObject::new();
    answer.insert(
        "later_chunk".to_owned(),
        Json::Bool(later_chunk_model(&check.chunks, check.assembled.as_ref())),
    );
    answer.insert(
        "client_named".to_owned(),
        Json::Bool(client_named_model(
            &check.request_data,
            check.assembled.as_ref(),
        )),
    );
    Json::Object(answer)
}
