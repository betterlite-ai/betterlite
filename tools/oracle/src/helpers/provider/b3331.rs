use betterlite_http::proxy::UpstreamResponse;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_provider::api::video_contracts::config::BaseVideoConfig as _;
use betterlite_provider::api::video_contracts::records::{
    Headers, Params, VideoLogging, VideoReply, VideoRequestContext,
};
use http::HeaderMap;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;

mod subject;
mod unsupported;

use subject::{Client, Subject};
use unsupported::{RequestOperation, ResponseOperation, unsupported_request, unsupported_response};

const API_BASE: &str = "https://mock/videos";

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    MultipartDefault,
    PrefetchDefault,
    AsyncContentFallback {
        body: String,
    },
    AsyncStatusFallback {
        body: String,
        custom_llm_provider: Option<String>,
    },
    UnsupportedRequest {
        operation: RequestOperation,
    },
    UnsupportedResponse {
        operation: ResponseOperation,
    },
    ErrorClass {
        error_message: String,
        status_code: u16,
        headers: Headers,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::MultipartDefault => Ok(json!({"value": Subject.use_multipart_form_data()})),
            Input::PrefetchDefault => Ok(prefetch_default()),
            Input::AsyncContentFallback { body } => async_content_fallback(body),
            Input::AsyncStatusFallback {
                body,
                custom_llm_provider,
            } => async_status_fallback(body, custom_llm_provider.as_deref()),
            Input::UnsupportedRequest { operation } => Ok(unsupported_request(*operation)),
            Input::UnsupportedResponse { operation } => Ok(unsupported_response(*operation)),
            Input::ErrorClass {
                error_message,
                status_code,
                headers,
            } => Ok(error_class(error_message, *status_code, headers)),
        }
    }
}

fn prefetch_default() -> serde_json::Value {
    let params = Params::new();
    let headers = Headers::new();
    let context = VideoRequestContext {
        api_base: API_BASE,
        litellm_params: &params,
        headers: &headers,
    };
    let prefetch = Subject.get_video_edit_prefetch_params("video_1", context);
    json!({"value": prefetch.map(|_| true)})
}

pub(super) fn response(body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn async_content_fallback(body: &str) -> HelperResult {
    let raw = response(body);
    let mut logging = VideoLogging::default();
    let content = Builder::new_current_thread()
        .build()?
        .block_on(Subject.async_transform_video_content_response(&raw, &mut logging))
        .or_value_error()?;
    let text = String::from_utf8(content).or_value_error()?;
    Ok(json!({"content": text}))
}

fn async_status_fallback(body: &str, custom_llm_provider: Option<&str>) -> HelperResult {
    let raw = response(body);
    let mut logging = VideoLogging::default();
    let reply = VideoReply {
        raw_response: &raw,
        logging_obj: &mut logging,
        custom_llm_provider,
    };
    let video = Builder::new_current_thread()
        .build()?
        .block_on(Subject.async_transform_video_status_retrieve_response(reply, Some(&Client)))
        .or_value_error()?;
    Ok(json!({"id": video.id, "status": video.status, "progress": video.progress}))
}

fn error_class(error_message: &str, status_code: u16, headers: &Headers) -> serde_json::Value {
    let error = Subject.get_error_class(error_message, status_code, headers);
    let sent: IndexMap<String, String> = error.headers.into_iter().flatten().collect();
    json!({"message": error.message, "status_code": error.status_code, "headers": sent})
}
