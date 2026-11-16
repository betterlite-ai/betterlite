use betterlite_http::proxy::UpstreamResponse;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_vertex::vertex_batch_lifecycle::page::decode_vertex_batch_page;
use http::header::HeaderMap;
use serde::Deserialize;
use serde_json::Value;

// the pinned reader runs on a body the case carries, with no provider call behind it
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    body: String,
}

pub(super) fn run(input: &Input) -> HelperResult {
    let reply = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: input.body.clone().into_bytes(),
    };
    decode_vertex_batch_page(&reply).or_value_error()?;
    serde_json::from_str(&input.body).or_value_error()
}
