use crate::helpers::HelperResult;
use async_trait::async_trait;
use betterlite_core::api::realtime_contracts::config::{
    BaseRealtimeConfig, BaseRealtimeHttpConfig, Environment, ErrorInfo, RealtimeUrl,
};
use betterlite_core::api::realtime_contracts::error::{
    BaseLlmException, HeaderMap, RealtimeContractError, WebSocketError,
};
use betterlite_core::api::realtime_contracts::events::{
    RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use betterlite_core::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Debug, Deserialize)]
pub(super) struct HooksInput {
    complete_urls: Vec<Option<String>>,
    pub(super) error: ErrorInput,
}
#[derive(Debug, Deserialize)]
pub(super) struct ErrorInput {
    pub(super) message: String,
    pub(super) status_code: u16,
    pub(super) headers: HeaderMap,
}
#[derive(Debug, Deserialize)]
pub(super) struct HttpUrlsInput {
    urls: Vec<UrlInput>,
}
#[derive(Debug, Deserialize)]
struct UrlInput {
    api_base: Option<String>,
    model: String,
    api_version: Option<String>,
    ephemeral_key: String,
}
struct EmptyBackend;
#[async_trait]
impl RealtimeBackend for EmptyBackend {
    async fn send(&mut self, _message: RealtimeMessage) -> Result<(), WebSocketError> {
        Ok(())
    }

    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, WebSocketError> {
        Ok(RealtimeMessage::Text(String::new()))
    }

    async fn close(&mut self) -> Result<(), WebSocketError> {
        Ok(())
    }
}
struct ContractConfig;
#[async_trait]
impl BaseRealtimeConfig for ContractConfig {
    type Error = RealtimeContractError;
    type Backend = EmptyBackend;
    fn validate_environment(
        &self,
        environment: &Environment<'_>,
    ) -> Result<HeaderMap, Self::Error> {
        Ok(environment.headers.clone())
    }
    fn transform_realtime_request(
        &self,
        _message: &str,
        _model: &str,
        _session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, Self::Error> {
        Ok(Vec::new())
    }
    fn transform_realtime_response(
        &self,
        _message: &RealtimeMessage,
        _model: &str,
        _input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, Self::Error> {
        Ok(RealtimeResponseTypedDict {
            response: Json::Null,
            current_output_item_id: None,
            current_response_id: None,
            current_delta_chunks: None,
            current_conversation_id: None,
            current_item_chunks: None,
            current_delta_type: None,
            session_configuration_request: None,
        })
    }
}
struct ContractHttpConfig;
impl BaseRealtimeHttpConfig for ContractHttpConfig {
    type Error = RealtimeContractError;
    fn get_api_base(&self, api_base: Option<&str>) -> Result<String, Self::Error> {
        Ok(api_base.unwrap_or_default().to_owned())
    }
    fn get_api_key(&self, api_key: Option<&str>) -> Result<String, Self::Error> {
        Ok(api_key.unwrap_or_default().to_owned())
    }
    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        _model: &str,
        _api_version: Option<&str>,
    ) -> String {
        api_base.unwrap_or_default().to_owned()
    }
    fn validate_environment(
        &self,
        environment: &Environment<'_>,
    ) -> Result<HeaderMap, Self::Error> {
        Ok(environment.headers.clone())
    }
}
fn error_record(exception: &BaseLlmException, raised: bool) -> Value {
    json!({
        "raised": raised,
        "status_code": exception.status_code,
        "message": exception.message,
        "headers": exception.headers,
    })
}

pub(super) fn hooks(input: &HooksInput) -> HelperResult {
    let config = ContractConfig;
    let http_config = ContractHttpConfig;
    let complete_urls: Vec<String> = input
        .complete_urls
        .iter()
        .map(|api_base| config.get_complete_url(api_base.as_deref(), "model", None))
        .collect();
    let error = ErrorInfo {
        message: &input.error.message,
        status_code: input.error.status_code,
        headers: &input.error.headers,
    };
    let websocket_error = match config.get_error_class(&error) {
        Ok(never) => match never {},
        Err(exception) => error_record(&exception, true),
    };
    let returned = http_config.get_error_class(&error);
    Ok(json!({
        "complete_urls": complete_urls,
        "session_configuration_request": config.session_configuration_request("model"),
        "session_close_usage": config.session_close_usage("model"),
        "session_created_event": config.session_created_event("model", "session", None),
        "is_setup_message": config.is_setup_message(&Json::Null),
        "is_content_message": config.is_content_message(&Json::Null),
        "requires_session_configuration": config.requires_session_configuration(),
        "websocket_error": websocket_error,
        "http_error": error_record(&returned, false),
    }))
}

pub(super) fn http_urls(input: &HttpUrlsInput) -> HelperResult {
    let config = ContractHttpConfig;
    let urls: Vec<Value> = input
        .urls
        .iter()
        .map(|document| {
            let headers = config.get_realtime_calls_headers(&document.ephemeral_key);
            let url = RealtimeUrl {
                api_base: document.api_base.as_deref(),
                model: &document.model,
                api_version: document.api_version.as_deref(),
            };
            json!({
                "api_base": document.api_base,
                "model": document.model,
                "api_version": document.api_version,
                "transcription": config.get_transcription_session_url(&url),
                "calls": config.get_realtime_calls_url(&url),
                "authorization": headers.get("Authorization"),
            })
        })
        .collect();
    Ok(json!({"urls": urls}))
}
