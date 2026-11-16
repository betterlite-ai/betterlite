use betterlite_http::proxy::UpstreamResponse;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_vertex::vertex_batch_lifecycle::page::decode_vertex_batch_page;
use http::header::HeaderMap;
use serde::Deserialize;
use serde_json::{Map, Value};

// the pinned reader runs on a body the case carries, with no provider call behind it
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    body: String,
}

// both sides project the decoded page to each job's name, state and createTime and to the token
pub(super) fn run(input: &Input) -> HelperResult {
    let reply = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: input.body.clone().into_bytes(),
    };
    let page = decode_vertex_batch_page(&reply).or_value_error()?;
    let jobs = match page.batch_prediction_jobs {
        None => Value::Null,
        Some(jobs) => Value::Array(
            jobs.into_iter()
                .map(|job| {
                    let mut fields = Map::new();
                    if let Some(name) = job.name {
                        fields.insert("name".to_owned(), Value::String(name));
                    }
                    if let Some(state) = job.state {
                        fields.insert("state".to_owned(), state.map_or(Value::Null, Value::String));
                    }
                    if let Some(create_time) = job.create_time {
                        fields.insert("createTime".to_owned(), Value::String(create_time));
                    }
                    Value::Object(fields)
                })
                .collect(),
        ),
    };
    let mut out = Map::new();
    out.insert("batchPredictionJobs".to_owned(), jobs);
    if let Some(token) = page.next_page_token {
        out.insert("nextPageToken".to_owned(), Value::String(token));
    }
    Ok(Value::Object(out))
}
