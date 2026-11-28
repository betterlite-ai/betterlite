use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::api::vector_store_http_write::service::{ClientSource, WriteTransport};
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageFiles, ImageHeaders, ImageParams, ImageResponse,
};
use betterlite_provider::api::image_contracts::edit::{
    BaseImageEditConfig, ImageEditEnvironment, ImageEditReply, ImageEditRequest,
};
use betterlite_provider::api::image_contracts::error::ImageContractError;
use betterlite_provider::api::image_edit_http::asynchronous::async_image_edit_handler;
use betterlite_provider::api::image_edit_http::error::ImageEditHttpError;
use betterlite_provider::api::image_edit_http::image_edit_handler;
use betterlite_provider::api::image_edit_http::service::{
    ClientKind, ImageEditCall, ImageEditDeps, ImageEditOutcome, LogContext,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use secrecy::ExposeSecret as _;
use thiserror::Error;
use url::Url;

use super::super::error::SdkHttpError;
use super::LoggedCall;

// The client the handler selects; the case env only carries the proxy settings
pub(super) struct Source(pub(super) ProxiedClient);

impl ClientSource for Source {
    type Client = ProxiedClient;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<ProxiedClient, ClientError> {
        Ok(self.0.clone())
    }
}

// The config's own refusals, kept apart from the provider's answer
#[derive(Debug, Error)]
pub(super) enum FixtureError {
    #[error("the image edit contract refused the call")]
    Contract(#[from] ImageContractError),
    #[error("the fixture's reply is not an image response")]
    Reply(#[source] serde_json::Error),
}

// The provider config a real deployment supplies: fixed transforms, the trusted URL and the finalizer
pub(super) struct FixtureConfig {
    pub(super) multipart: bool,
}

#[async_trait]
impl BaseImageEditConfig for FixtureConfig {
    type Error = FixtureError;

    fn get_supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_openai_params(
        &self,
        _image_edit_optional_params: &ImageParams,
        _model: &str,
        _drop_params: bool,
    ) -> ImageParams {
        ImageParams::new()
    }

    fn transform_image_edit_request(
        &self,
        request: ImageEditRequest<'_>,
    ) -> Result<(ImageParams, ImageFiles), Self::Error> {
        let mut data = ImageParams::new();
        if let Some(prompt) = request.prompt {
            data.insert(String::from("prompt"), Json::Text(prompt.to_owned()));
        }
        let files: ImageFiles = request
            .image
            .map(|image| vec![(String::from("image"), image.clone())])
            .unwrap_or_default();
        Ok((data, files))
    }

    fn transform_image_edit_response(
        &self,
        reply: ImageEditReply<'_>,
    ) -> Result<ImageResponse, Self::Error> {
        serde_json::from_slice(&reply.raw_response.body).map_err(FixtureError::Reply)
    }

    fn validate_environment(&self, request: ImageEditEnvironment<'_>) -> ImageHeaders {
        let mut headers = ImageHeaders::new();
        headers.insert(String::from("x"), String::from("validated"));
        if let Some(key) = request.api_key {
            headers.insert(
                String::from("authorization"),
                format!("Bearer {}", key.expose_secret()),
            );
        }
        headers
    }

    fn get_complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        _litellm_params: &ImageParams,
    ) -> Result<String, Self::Error> {
        let Some(base) = api_base else {
            return Err(FixtureError::Contract(ImageContractError::ApiBaseRequired));
        };
        Ok(format!("{base}/images/edits"))
    }

    // ast-grep-ignore: no-long-names
    fn finalize_image_edit_request_data(
        &self,
        mut data: ImageParams,
        _resolved_request_url: &str,
    ) -> ImageParams {
        data.insert(String::from("revision"), Json::Text(String::from("final")));
        data
    }

    fn use_multipart_form_data(&self) -> bool {
        self.multipart
    }
}

pub(super) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// The pre-call a production request runs: the same state a request would log through, with nothing attached to it
pub(super) struct Fixture {
    redactor: Redactor,
    sink: Sink,
    state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(super) fn new() -> Result<Self, SdkHttpError> {
        let init: LoggingInit = serde_json::from_str(
            r#"{"model":"gpt-image-1","call_type":"image_edit","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
        )
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
        let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
        let state = LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("generated"),
            Duration::default,
        )
        .map_err(SdkHttpError::LoggingState)?;
        Ok(Self {
            redactor,
            sink: Sink,
            state,
            now,
        })
    }

    pub(super) async fn run<C, S>(
        &mut self,
        mode: ClientKind,
        deps: &ImageEditDeps<'_, C, S>,
        call: &ImageEditCall<'_>,
    ) -> Result<ImageEditOutcome, ImageEditHttpError<C::Error>>
    where
        C: BaseImageEditConfig,
        S: ClientSource<Client: WriteTransport + Clone>,
    {
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: self.now,
        };
        let mut log = LogContext {
            state: &mut self.state,
            env: &env,
        };
        match mode {
            ClientKind::Sync => image_edit_handler(deps, call, &mut log).await,
            ClientKind::Asynchronous => async_image_edit_handler(deps, call, &mut log).await,
        }
    }

    // What the handler logged, in the shape both sides can compare: the prompt, the per-mode body, the header names and the URL's path
    pub(super) fn logged(&self) -> Option<LoggedCall> {
        let details = &self.state.model_call_details;
        let input = details.get("input").cloned().unwrap_or(Json::Null);
        let Json::Object(args) = details.get("additional_args")? else {
            return None;
        };
        let complete_input_dict = args
            .get("complete_input_dict")
            .cloned()
            .unwrap_or(Json::Null);
        let mut header_names: Vec<String> = match args.get("headers") {
            Some(Json::Object(headers)) => headers.keys().cloned().collect(),
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
            | None => Vec::new(),
        };
        header_names.sort();
        let api_base = match args.get("api_base") {
            Some(Json::Text(text)) => text.as_str(),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => "",
        };
        Some(LoggedCall {
            input,
            complete_input_dict,
            header_names,
            api_base_path: Url::parse(api_base)
                .map_or_else(|_| String::new(), |url| url.path().to_owned()),
        })
    }
}
