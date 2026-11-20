use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::records::{Headers, StatusRequest};

// pre_call(input="", api_key="", additional_args={api_base: url, headers, video_id, data}); B249 keeps no api_key field
pub(crate) fn pre_call_record(request: &StatusRequest, video_id: &str) -> PreCall {
    let mut passthrough = IndexMap::new();
    passthrough.insert("video_id".to_owned(), Json::Text(video_id.to_owned()));
    // the data key stays present with an empty map, as LiteLLM logs the transform's dict
    let data = request.body.clone().unwrap_or_default();
    passthrough.insert("data".to_owned(), Json::Object(data));
    PreCall {
        input: Json::Text(String::new()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(request.url.clone()),
            headers: Some(headers_json(&request.headers)),
            complete_input_dict: None,
            request_str: None,
            openai_sdk: None,
            passthrough,
        },
    }
}

fn headers_json(headers: &Headers) -> IndexMap<String, Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}
