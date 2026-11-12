use super::error::{BaseLlmException, HeaderMap};
use super::events::{
    AudioTranscriptionUsage, RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use super::transport::{RealtimeBackend, RealtimeMessage};
use async_trait::async_trait;
use betterlite_models::decisions::Json;
use std::convert::Infallible;
use std::error::Error as StdError;

pub struct Environment<'env> {
    pub headers: &'env HeaderMap,
    pub model: &'env str,
    pub api_key: Option<&'env str>,
}

pub struct ErrorInfo<'error> {
    pub message: &'error str,
    pub status_code: u16,
    pub headers: &'error HeaderMap,
}

pub struct RealtimeUrl<'url> {
    pub api_base: Option<&'url str>,
    pub model: &'url str,
    pub api_version: Option<&'url str>,
}

#[async_trait]
pub trait BaseRealtimeConfig: Send + Sync {
    type Error: StdError + Send + Sync + 'static;
    type Backend: RealtimeBackend;
    fn validate_environment(&self, environment: &Environment<'_>)
    -> Result<HeaderMap, Self::Error>;
    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        _model: &str,
        _api_key: Option<&str>,
    ) -> String {
        api_base.unwrap_or_default().to_owned()
    }

    // LiteLLM's WebSocket base method raises, so it never hands a value back
    fn get_error_class(&self, error: &ErrorInfo<'_>) -> Result<Infallible, BaseLlmException> {
        Err(BaseLlmException {
            status_code: error.status_code,
            message: error.message.to_owned(),
            headers: error.headers.clone(),
        })
    }
    fn transform_realtime_request(
        &self,
        message: &str,
        model: &str,
        session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, Self::Error>;

    async fn pace_backend_send(&self, _message: &[u8]) {}
    fn is_setup_message(&self, _message: &Json) -> bool {
        false
    }
    fn is_content_message(&self, _message: &Json) -> bool {
        false
    }
    fn requires_session_configuration(&self) -> bool {
        false
    }
    fn session_configuration_request(&self, _model: &str) -> Option<String> {
        None
    }
    fn session_close_usage(&self, _model: &str) -> Option<AudioTranscriptionUsage> {
        None
    }

    async fn open_backend(
        &self,
        _url: &str,
        _headers: &HeaderMap,
    ) -> Result<Option<Self::Backend>, Self::Error> {
        Ok(None)
    }
    fn session_created_event(
        &self,
        _model: &str,
        _logging_session_id: &str,
        _session_configuration_request: Option<&str>,
    ) -> Option<Json> {
        None
    }
    fn transform_realtime_response(
        &self,
        message: &RealtimeMessage,
        model: &str,
        input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, Self::Error>;
}
pub trait BaseRealtimeHttpConfig: Send + Sync {
    type Error: StdError + Send + Sync + 'static;
    fn get_api_base(&self, api_base: Option<&str>) -> Result<String, Self::Error>;
    fn get_api_key(&self, api_key: Option<&str>) -> Result<String, Self::Error>;
    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        model: &str,
        api_version: Option<&str>,
    ) -> String;
    fn validate_environment(&self, environment: &Environment<'_>)
    -> Result<HeaderMap, Self::Error>;
    fn get_transcription_session_url(&self, url: &RealtimeUrl<'_>) -> String {
        let trimmed = url.api_base.unwrap_or_default().trim_end_matches('/');
        let base = trimmed.strip_suffix("/v1").unwrap_or(trimmed);
        format!("{base}/v1/realtime/transcription_sessions")
    }
    fn get_realtime_calls_url(&self, url: &RealtimeUrl<'_>) -> String {
        let base = url.api_base.unwrap_or_default().trim_end_matches('/');
        format!("{base}/v1/realtime/calls")
    }
    fn get_realtime_calls_headers(&self, ephemeral_key: &str) -> HeaderMap {
        HeaderMap::from([(
            "Authorization".to_owned(),
            Json::Text(format!("Bearer {ephemeral_key}")),
        )])
    }
    fn get_error_class(&self, error: &ErrorInfo<'_>) -> BaseLlmException {
        BaseLlmException {
            status_code: error.status_code,
            message: error.message.to_owned(),
            headers: error.headers.clone(),
        }
    }
}
