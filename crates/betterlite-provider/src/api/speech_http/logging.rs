use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::api::speech_provider_contract::TextToSpeechRequestData;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;

use super::LogContext;
use super::error::SpeechError;
use crate::api::speech_provider_contract::SpeechHeaders;

// The pre-call before the send: the input, the completed URL, the final headers and the transformed request data
pub(super) fn log_pre_call(
    input: &str,
    api_base: &str,
    headers: &SpeechHeaders,
    data: &TextToSpeechRequestData,
    log: &mut LogContext<'_, '_>,
) -> Result<(), SpeechError> {
    let record = PreCall {
        input: Json::Text(input.to_owned()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(api_base.to_owned()),
            headers: Some(logged_headers(headers)),
            complete_input_dict: Some(RawBody::Object(request_fields(data)?)),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env)?;
    Ok(())
}

fn logged_headers(headers: &SpeechHeaders) -> IndexMap<String, Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}

// request_data holds only the keys the config put in it, so a missing body logs no body key at all
fn request_fields(data: &TextToSpeechRequestData) -> Result<IndexMap<String, Json>, SpeechError> {
    let mut fields = IndexMap::new();
    if let Some(body) = &data.dict_body {
        fields.insert("dict_body".to_owned(), object_json(body)?);
    }
    if let Some(text) = &data.ssml_body {
        fields.insert("ssml_body".to_owned(), Json::Text(text.clone()));
    }
    if let Some(headers) = &data.headers {
        fields.insert("headers".to_owned(), Json::Object(logged_headers(headers)));
    }
    Ok(fields)
}

fn object_json(body: &Object) -> Result<Json, SpeechError> {
    serde_json::to_value(body)
        .and_then(serde_json::from_value::<Json>)
        .map_err(SpeechError::Encode)
}
