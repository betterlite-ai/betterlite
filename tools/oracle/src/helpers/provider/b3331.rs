use std::string::FromUtf8Error;

use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_core::api::video_records::{VideoCreateOptionalRequestParams, VideoObject};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_provider::api::video_contracts::config::BaseVideoConfig;
use betterlite_provider::api::video_contracts::error::VideoContractError;
use betterlite_provider::api::video_contracts::records::{
    Headers, Params, RequestFiles, VideoEditInput, VideoExtensionInput, VideoListQuery,
    VideoLogging, VideoReply, VideoRequestContext,
};
use http::HeaderMap;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;
use tokio::runtime::Builder;

const API_BASE: &str = "https://mock/videos";

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    MultipartDefault,
    PrefetchDefault,
    CompleteUrl {
        api_bases: Vec<Option<String>>,
    },
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

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RequestOperation {
    CreateCharacter,
    GetCharacter,
    Edit,
    Extension,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseOperation {
    CreateCharacter,
    GetCharacter,
    Edit,
    Extension,
}

#[derive(Debug, Error)]
enum SubjectError {
    #[error(transparent)]
    Contract(#[from] VideoContractError),
    #[error("the response body is not text")]
    Body(#[from] FromUtf8Error),
}

struct Client;

struct Subject;

fn video_from(reply: &VideoReply<'_>, status: &str) -> Result<VideoObject, SubjectError> {
    let id = String::from_utf8(reply.raw_response.body.clone())?;
    Ok(VideoObject::new(id, status.to_owned()))
}

impl BaseVideoConfig for Subject {
    type Error = SubjectError;
    type SyncClient = Client;
    type AsyncClient = Client;

    fn get_supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_openai_params(
        &self,
        _video_create_optional_params: &VideoCreateOptionalRequestParams,
        _model: &str,
        _drop_params: bool,
    ) -> Params {
        Params::new()
    }

    fn transform_video_create_request(
        &self,
        _model: &str,
        _prompt: &str,
        _optional_params: &VideoCreateOptionalRequestParams,
        context: VideoRequestContext<'_>,
    ) -> Result<(Params, RequestFiles, String), SubjectError> {
        Ok((Params::new(), Vec::new(), context.api_base.to_owned()))
    }

    fn transform_video_create_response(
        &self,
        _model: &str,
        reply: VideoReply<'_>,
        _request_data: Option<&Params>,
    ) -> Result<VideoObject, SubjectError> {
        video_from(&reply, "queued")
    }

    fn transform_video_content_request(
        &self,
        _video_id: &str,
        _variant: Option<&str>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), SubjectError> {
        Ok((context.api_base.to_owned(), Params::new()))
    }

    fn transform_video_content_response(
        &self,
        raw_response: &UpstreamResponse,
        _logging_obj: &mut VideoLogging,
    ) -> Result<Vec<u8>, SubjectError> {
        Ok(raw_response.body.clone())
    }

    fn transform_video_remix_request(
        &self,
        _video_id: &str,
        _prompt: &str,
        _extra_body: Option<&Params>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), SubjectError> {
        Ok((context.api_base.to_owned(), Params::new()))
    }

    fn transform_video_remix_response(
        &self,
        reply: VideoReply<'_>,
    ) -> Result<VideoObject, SubjectError> {
        video_from(&reply, "queued")
    }

    fn transform_video_list_request(
        &self,
        _query: VideoListQuery<'_>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), SubjectError> {
        Ok((context.api_base.to_owned(), Params::new()))
    }

    fn transform_video_list_response(
        &self,
        _reply: VideoReply<'_>,
    ) -> Result<Params, SubjectError> {
        Ok(Params::new())
    }

    fn transform_video_status_retrieve_request(
        &self,
        _video_id: &str,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), SubjectError> {
        Ok((context.api_base.to_owned(), Params::new()))
    }

    fn transform_video_status_retrieve_response(
        &self,
        reply: VideoReply<'_>,
        client: Option<&Client>,
    ) -> Result<VideoObject, SubjectError> {
        let status = reply.custom_llm_provider.unwrap_or("none");
        let mut video = video_from(&reply, status)?;
        video.progress = Some(i64::from(client.is_some()));
        Ok(video)
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::MultipartDefault => Ok(json!({"value": Subject.use_multipart_form_data()})),
            Input::PrefetchDefault => Ok(prefetch_default()),
            Input::CompleteUrl { api_bases } => Ok(complete_url(api_bases)),
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

fn complete_url(api_bases: &[Option<String>]) -> serde_json::Value {
    let params = Params::new();
    let urls: Vec<serde_json::Value> = api_bases
        .iter()
        .map(
            |api_base| match Subject.get_complete_url("m", api_base.as_deref(), &params) {
                Ok(url) => json!({"url": url, "error": null}),
                Err(error) => json!({"url": null, "error": error.to_string()}),
            },
        )
        .collect();
    json!({"urls": urls})
}

fn response(body: &str) -> UpstreamResponse {
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

fn unsupported_request(operation: RequestOperation) -> serde_json::Value {
    let params = Params::new();
    let headers = Headers::new();
    let context = VideoRequestContext {
        api_base: API_BASE,
        litellm_params: &params,
        headers: &headers,
    };
    let outcome = match operation {
        RequestOperation::CreateCharacter => Subject
            .transform_video_create_character_request(
                "name",
                &SuppliedMedia::Bytes(Vec::new()),
                context,
            )
            .map(drop),
        RequestOperation::GetCharacter => Subject
            .transform_video_get_character_request("char_1", context)
            .map(drop),
        RequestOperation::Edit => {
            let input = VideoEditInput {
                prompt: "prompt",
                video_id: "video_1",
                video_file: None,
                extra_body: None,
                prefetched_source_data: None,
            };
            Subject
                .transform_video_edit_request(input, context)
                .map(drop)
        }
        RequestOperation::Extension => {
            let input = VideoExtensionInput {
                prompt: "prompt",
                video_id: "video_1",
                seconds: "4",
                extra_body: None,
            };
            Subject
                .transform_video_extension_request(input, context)
                .map(drop)
        }
    };
    raised(outcome)
}

fn unsupported_response(operation: ResponseOperation) -> serde_json::Value {
    let raw = response("");
    let mut logging = VideoLogging::default();
    let reply = VideoReply {
        raw_response: &raw,
        logging_obj: &mut logging,
        custom_llm_provider: None,
    };
    let outcome = match operation {
        ResponseOperation::CreateCharacter => Subject
            .transform_video_create_character_response(reply.raw_response, reply.logging_obj)
            .map(drop),
        ResponseOperation::GetCharacter => Subject
            .transform_video_get_character_response(reply.raw_response, reply.logging_obj)
            .map(drop),
        ResponseOperation::Edit => Subject.transform_video_edit_response(reply, None).map(drop),
        ResponseOperation::Extension => Subject.transform_video_extension_response(reply).map(drop),
    };
    raised(outcome)
}

fn raised(outcome: Result<(), SubjectError>) -> serde_json::Value {
    match outcome {
        Ok(()) => json!({"raised": false, "message": ""}),
        Err(error) => json!({"raised": true, "message": error.to_string()}),
    }
}

fn error_class(error_message: &str, status_code: u16, headers: &Headers) -> serde_json::Value {
    let error = Subject.get_error_class(error_message, status_code, headers);
    let sent: IndexMap<String, String> = error.headers.into_iter().flatten().collect();
    json!({"message": error.message, "status_code": error.status_code, "headers": sent})
}
