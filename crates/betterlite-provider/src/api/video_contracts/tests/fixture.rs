#![cfg(test)]

use std::string::FromUtf8Error;

use betterlite_core::api::video_records::{VideoCreateOptionalRequestParams, VideoObject};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use thiserror::Error;

use crate::api::video_contracts::config::BaseVideoConfig;
use crate::api::video_contracts::error::VideoContractError;
use crate::api::video_contracts::records::{
    Headers, Params, RequestFiles, VideoListQuery, VideoLogging, VideoReply, VideoRequestContext,
};

#[derive(Debug, Error)]
pub(super) enum FixtureError {
    #[error(transparent)]
    Contract(#[from] VideoContractError),
    #[error("the fixture refused status {0}")]
    Refused(u16),
    #[error("the fixture body is not text")]
    Body(#[from] FromUtf8Error),
}

pub(super) struct SyncHandle;

pub(super) struct AsyncHandle;

// Implements only what LiteLLM declares abstract, so every other member runs the trait default
pub(super) struct Subject;

#[derive(Default)]
pub(super) struct Parts {
    params: Params,
    headers: Headers,
}

impl Parts {
    pub(super) fn context(&self) -> VideoRequestContext<'_> {
        VideoRequestContext {
            api_base: "https://mock/videos",
            litellm_params: &self.params,
            headers: &self.headers,
        }
    }
}

pub(super) fn raw(status: u16, body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn refuse_failure(raw_response: &UpstreamResponse) -> Result<(), FixtureError> {
    if raw_response.status >= 400 {
        return Err(FixtureError::Refused(raw_response.status));
    }
    Ok(())
}

fn queued(reply: &VideoReply<'_>) -> Result<VideoObject, FixtureError> {
    let id = String::from_utf8(reply.raw_response.body.clone())?;
    Ok(VideoObject::new(id, "queued".to_owned()))
}

impl BaseVideoConfig for Subject {
    type Error = FixtureError;
    type SyncClient = SyncHandle;
    type AsyncClient = AsyncHandle;

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
        model: &str,
        prompt: &str,
        optional_params: &Params,
        context: VideoRequestContext<'_>,
    ) -> Result<(Params, RequestFiles, String), FixtureError> {
        let mut data = optional_params.clone();
        data.insert("model".to_owned(), Json::Text(model.to_owned()));
        data.insert("prompt".to_owned(), Json::Text(prompt.to_owned()));
        Ok((data, Vec::new(), context.api_base.to_owned()))
    }

    fn transform_video_create_response(
        &self,
        _model: &str,
        reply: VideoReply<'_>,
        _request_data: Option<&Params>,
    ) -> Result<VideoObject, FixtureError> {
        queued(&reply)
    }

    fn transform_video_content_request(
        &self,
        _video_id: &str,
        _variant: Option<&str>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), FixtureError> {
        Ok((context.api_base.to_owned(), Params::new()))
    }

    fn transform_video_content_response(
        &self,
        raw_response: &UpstreamResponse,
        logging_obj: &mut VideoLogging,
    ) -> Result<Vec<u8>, FixtureError> {
        refuse_failure(raw_response)?;
        logging_obj
            .model_call_details
            .insert("content_seen".to_owned(), Json::Bool(true));
        Ok(raw_response.body.clone())
    }

    fn transform_video_remix_request(
        &self,
        video_id: &str,
        prompt: &str,
        extra_body: Option<&Params>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), FixtureError> {
        let mut data = Params::from([
            ("prompt".to_owned(), Json::Text(prompt.to_owned())),
            ("video_id".to_owned(), Json::Text(video_id.to_owned())),
        ]);
        if let Some(extra) = extra_body {
            data.extend(extra.clone());
        }
        Ok((context.api_base.to_owned(), data))
    }

    fn transform_video_remix_response(
        &self,
        reply: VideoReply<'_>,
    ) -> Result<VideoObject, FixtureError> {
        queued(&reply)
    }

    fn transform_video_list_request(
        &self,
        _query: VideoListQuery<'_>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), FixtureError> {
        Ok((context.api_base.to_owned(), Params::new()))
    }

    fn transform_video_list_response(
        &self,
        _reply: VideoReply<'_>,
    ) -> Result<Params, FixtureError> {
        Ok(Params::new())
    }

    fn transform_video_status_retrieve_request(
        &self,
        _video_id: &str,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), FixtureError> {
        Ok((context.api_base.to_owned(), Params::new()))
    }

    fn transform_video_status_retrieve_response(
        &self,
        reply: VideoReply<'_>,
        client: Option<&SyncHandle>,
    ) -> Result<VideoObject, FixtureError> {
        refuse_failure(reply.raw_response)?;
        reply
            .logging_obj
            .model_call_details
            .insert("status_seen".to_owned(), Json::Bool(true));
        let mut video = queued(&reply)?;
        video.status = reply.custom_llm_provider.unwrap_or("none").to_owned();
        video.progress = Some(i64::from(client.is_some()));
        Ok(video)
    }
}
