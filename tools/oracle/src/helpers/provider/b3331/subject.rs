use std::string::FromUtf8Error;

use betterlite_core::api::video_records::VideoObject;
// ast-grep-ignore: no-long-names
use betterlite_core::api::video_records::VideoCreateOptionalRequestParams;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_provider::api::video_contracts::config::BaseVideoConfig;
use betterlite_provider::api::video_contracts::error::VideoContractError;
use betterlite_provider::api::video_contracts::records::{
    Params, RequestFiles, VideoListQuery, VideoLogging, VideoReply, VideoRequestContext,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub(super) enum SubjectError {
    #[error(transparent)]
    Contract(#[from] VideoContractError),
    #[error("the response body is not text")]
    Body(#[from] FromUtf8Error),
}

pub(super) struct Client;

pub(super) struct Subject;

impl BaseVideoConfig for Subject {
    type Error = SubjectError;
    type SyncClient = Client;
    type AsyncClient = Client;

    fn get_supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_openai_params(
        &self,
        // ast-grep-ignore: no-long-names
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
        _optional_params: &Params,
        context: VideoRequestContext<'_>,
    ) -> Result<(Params, RequestFiles, String), SubjectError> {
        Ok((Params::new(), Vec::new(), context.api_base.to_owned()))
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_create_response(
        &self,
        _model: &str,
        reply: VideoReply<'_>,
        _request_data: Option<&Params>,
    ) -> Result<VideoObject, SubjectError> {
        let id = String::from_utf8(reply.raw_response.body.clone())?;
        Ok(VideoObject::new(id, "queued".to_owned()))
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_content_request(
        &self,
        _video_id: &str,
        _variant: Option<&str>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), SubjectError> {
        Ok((context.api_base.to_owned(), Params::new()))
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_content_response(
        &self,
        raw_response: &UpstreamResponse,
        _logging_obj: &mut VideoLogging,
    ) -> Result<Vec<u8>, SubjectError> {
        Ok(raw_response.body.clone())
    }

    fn transform_video_remix_request(
        &self,
        video_id: &str,
        prompt: &str,
        _extra_body: Option<&Params>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), SubjectError> {
        let mut data = Params::new();
        data.insert("prompt".to_owned(), Json::Text(prompt.to_owned()));
        data.insert("video_id".to_owned(), Json::Text(video_id.to_owned()));
        Ok((context.api_base.to_owned(), data))
    }

    fn transform_video_remix_response(
        &self,
        reply: VideoReply<'_>,
    ) -> Result<VideoObject, SubjectError> {
        let id = String::from_utf8(reply.raw_response.body.clone())?;
        Ok(VideoObject::new(id, "queued".to_owned()))
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

    // ast-grep-ignore: no-long-names
    fn transform_video_status_retrieve_request(
        &self,
        _video_id: &str,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), SubjectError> {
        Ok((context.api_base.to_owned(), Params::new()))
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_status_retrieve_response(
        &self,
        reply: VideoReply<'_>,
        client: Option<&Client>,
    ) -> Result<VideoObject, SubjectError> {
        let id = String::from_utf8(reply.raw_response.body.clone())?;
        let status = reply.custom_llm_provider.unwrap_or("none");
        let mut video = VideoObject::new(id, status.to_owned());
        video.progress = Some(i64::from(client.is_some()));
        Ok(video)
    }
}
