#![cfg(test)]

use std::sync::{Mutex, PoisonError};

use betterlite_http::proxy::UpstreamResponse;
use thiserror::Error;

use super::providers::VideoGenerationConfig;
use crate::api::video_records::{VideoCreateOptionalRequestParams, VideoObject};
use crate::api::videos_http_transport::video_config::{
    Params, RequestFiles, VideoHttpConfig, VideoLogging, VideoReply, VideoRequestContext,
};

#[derive(Debug, Error)]
#[error("the testing config only maps options")]
pub(crate) struct TestingError;

// one map_openai_params call, kept so a test can check the model and the drop-params switch it was handed
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MapCall {
    pub(crate) model: String,
    pub(crate) drop_params: bool,
    pub(crate) options: VideoCreateOptionalRequestParams,
}

// a provider config whose mapping answer the test decides, with the real trait's transport half unused
#[derive(Debug, Default)]
pub(crate) struct TestingConfig {
    mapped: Mutex<Option<Params>>,
    calls: Mutex<Vec<MapCall>>,
}

impl TestingConfig {
    pub(crate) fn returning(mapped: Params) -> Self {
        Self {
            mapped: Mutex::new(Some(mapped)),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<MapCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl VideoGenerationConfig for TestingConfig {
    fn map_openai_params(
        &self,
        options: &VideoCreateOptionalRequestParams,
        model: &str,
        drop_params: bool,
    ) -> Params {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(MapCall {
                model: model.to_owned(),
                drop_params,
                options: options.clone(),
            });
        self.mapped
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .unwrap_or_default()
    }
}

impl VideoHttpConfig for TestingConfig {
    type Error = TestingError;

    fn get_complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, TestingError> {
        Ok(api_base.unwrap_or("http://testing.test/videos").to_owned())
    }

    fn transform_video_create_request(
        &self,
        _model: &str,
        _prompt: &str,
        _optional_params: &Params,
        _context: VideoRequestContext<'_>,
    ) -> Result<(Params, RequestFiles, String), TestingError> {
        Err(TestingError)
    }

    fn transform_video_create_response(
        &self,
        _model: &str,
        _reply: VideoReply<'_>,
        _request_data: Option<&Params>,
    ) -> Result<VideoObject, TestingError> {
        Err(TestingError)
    }

    fn transform_video_content_request(
        &self,
        _video_id: &str,
        _variant: Option<&str>,
        _context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), TestingError> {
        Err(TestingError)
    }

    fn transform_video_content_response(
        &self,
        _raw_response: &UpstreamResponse,
        _logging_obj: &mut VideoLogging,
    ) -> Result<Vec<u8>, TestingError> {
        Err(TestingError)
    }
}
