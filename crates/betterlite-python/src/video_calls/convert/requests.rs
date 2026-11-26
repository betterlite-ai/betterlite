use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_core::api::video_content_call::VideoContentRequest;
use betterlite_core::api::video_generation_call::VideoGenerationRequest;
use betterlite_core::api::video_list_call::VideoListCall;
use betterlite_core::api::video_status_call::VideoStatusRequest;
use betterlite_models::decisions::Json;
use betterlite_sdk::call_setup::RequestMap;
use secrecy::SecretString;

use super::fields::{
    INTEGER, STRING, duration, failed, meta, number, params, seconds, text, texts,
};
use super::{
    CONTENT_NAMED, ConvertError, GENERATION_NAMED, LIST_NAMED, Operation, Reference, Requests,
    STATUS_NAMED, loose,
};

pub(super) fn generation(
    arguments: &RequestMap,
    media: Option<Reference>,
) -> Result<VideoGenerationRequest, ConvertError> {
    let mut generation = VideoGenerationRequest::new(
        text(arguments, "prompt")?.ok_or_else(|| failed("prompt", STRING))?,
    );
    generation.model = text(arguments, "model")?;
    generation.seconds = seconds(arguments, "seconds")?;
    generation.size = text(arguments, "size")?;
    generation.user = text(arguments, "user")?;
    generation.timeout = number(arguments, "timeout")?;
    generation.custom_llm_provider = text(arguments, "custom_llm_provider")?;
    generation.extra_headers = texts(arguments, "extra_headers")?;
    generation.extra_query = params(arguments, "extra_query")?;
    generation.extra_body = params(arguments, "extra_body")?;
    generation.input_reference = media.map(|reference| SuppliedMedia::Bytes(reference.content));
    generation.kwargs = loose(arguments, &GENERATION_NAMED);
    Ok(generation)
}

pub(super) fn list(arguments: &RequestMap) -> Result<VideoListCall, ConvertError> {
    Ok(VideoListCall {
        after: text(arguments, "after")?,
        limit: match arguments.get("limit") {
            None | Some(Json::Null) => None,
            Some(Json::Number(value)) => {
                Some(value.as_i64().ok_or_else(|| failed("limit", INTEGER))?)
            }
            Some(_) => return Err(failed("limit", INTEGER)),
        },
        order: text(arguments, "order")?,
        custom_llm_provider: text(arguments, "custom_llm_provider")?,
        api_key: text(arguments, "api_key")?.map(SecretString::from),
        api_base: text(arguments, "api_base")?,
        api_version: text(arguments, "api_version")?,
        extra_headers: texts(arguments, "extra_headers")?,
        extra_query: params(arguments, "extra_query")?,
        extra_body: params(arguments, "extra_body")?,
        mock_response: arguments.get("mock_response").cloned(),
        litellm_metadata: meta(arguments, "litellm_metadata")?,
        litellm_call_id: text(arguments, "litellm_call_id")?,
        user: text(arguments, "user")?,
        timeout: number(arguments, "timeout")?,
        extra: loose(arguments, &LIST_NAMED),
    })
}

pub(super) fn read(operation: Operation, arguments: &RequestMap) -> Result<Requests, ConvertError> {
    let video_id = text(arguments, "video_id")?.ok_or_else(|| failed("video_id", STRING))?;
    let timeout = duration(number(arguments, "timeout")?)?;
    let custom_llm_provider = text(arguments, "custom_llm_provider")?;
    let extra_headers = texts(arguments, "extra_headers")?;
    let extra_query = params(arguments, "extra_query")?;
    let extra_body = params(arguments, "extra_body")?;
    let mut requests = Requests::default();
    match operation {
        Operation::Content => {
            requests.content = Some(VideoContentRequest {
                video_id,
                timeout,
                custom_llm_provider,
                variant: text(arguments, "variant")?,
                extra_headers,
                extra_query,
                extra_body,
                kwargs: loose(arguments, &CONTENT_NAMED),
            });
        }
        Operation::Status => {
            requests.status = Some(VideoStatusRequest {
                video_id,
                timeout: Some(timeout.unwrap_or_default()),
                custom_llm_provider,
                extra_headers,
                extra_query,
                extra_body,
                kwargs: loose(arguments, &STATUS_NAMED),
            });
        }
        Operation::Generate | Operation::Remix | Operation::List => return Err(ConvertError::Kind),
    }
    Ok(requests)
}
