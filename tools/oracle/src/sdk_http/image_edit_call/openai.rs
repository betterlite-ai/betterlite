use std::time::Duration;

use betterlite_core::api::image_edit_call::call::ImageEditStep;
use betterlite_core::api::image_edit_call::error::{ImageEditError, ProviderFailure};
use betterlite_core::api::image_edit_call::send::{ImageEditProvider, ImageEditSendRequest};
use betterlite_core::api::image_input_options::options::ImageEditOptionalRequestParams;
use betterlite_core::api::image_input_options::selection::{
    OptionalParamsPolicy, optional_params_image_edit,
};
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::api::vector_store_http_write::service::ClientSource;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_openai::api::image_contracts::records::{
    ImageFile, ImageHeaders, ImageParams, ImageResponse,
};
use betterlite_openai::image_edits::family::{SUPPORTED_OPENAI_PARAMS, select_image_edit_family};
use betterlite_openai::image_edits::file::ImageEditFile;
use betterlite_provider::api::image_edit_http::asynchronous::async_image_edit_handler;
use betterlite_provider::api::image_edit_http::error::ImageEditHttpError;
use betterlite_provider::api::image_edit_http::image_edit_handler;
use betterlite_provider::api::image_edit_http::service::{
    ClientKind, ImageEditCall as HandlerCall, ImageEditDeps, LogContext,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;

use self::config::OpenAiConfig;
use super::providers::Options;

mod config;

const LOGGING_INIT: &str = r#"{"model":"gpt-image-1","call_type":"image_edit","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

// OpenAIImageEditConfig over B2451's generic handler: B512's transforms, the trusted base and one pooled client
pub(super) struct OpenAiProvider {
    pub(super) client: ProxiedClient,
}

impl ImageEditProvider for OpenAiProvider {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        SUPPORTED_OPENAI_PARAMS.map(str::to_owned).to_vec()
    }

    fn use_multipart_form_data(&self) -> bool {
        true
    }

    fn map_edit_options(
        &self,
        options: &ImageEditOptionalRequestParams,
        model: &str,
        policy: &OptionalParamsPolicy,
    ) -> Result<ImageParams, ImageEditError> {
        let supported = Options {
            supported: &SUPPORTED_OPENAI_PARAMS,
        };
        optional_params_image_edit(&supported, model, options, policy)
    }

    // LiteLLM hands back the handler's answer or its coroutine; both are one future here
    fn send(&self, request: ImageEditSendRequest<'_>) -> Result<ImageEditStep<'_>, ImageEditError> {
        let seconds = request.timeout.unwrap_or_default();
        let timeout = Duration::try_from_secs_f64(seconds)
            .map_err(|_invalid| ImageEditError::InvalidTimeout { seconds })?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|error| ImageEditError::Invalid(error.to_string()))?;
        let edit = OwnedEdit {
            redactor,
            config: OpenAiConfig {
                family: select_image_edit_family(request.model),
                images: request.images.iter().map(nameless).collect(),
                mask: request.mask.map(nameless),
            },
            model: request.model.to_owned(),
            prompt: request.prompt.map(str::to_owned),
            optional_params: request.optional_params.clone(),
            litellm_params: request.litellm_params.clone(),
            extra_headers: request.extra_headers.cloned(),
            timeout,
            mode: if request.asynchronous {
                ClientKind::Asynchronous
            } else {
                ClientKind::Sync
            },
        };
        Ok(ImageEditStep::Pending(Box::pin(send_edit(
            self.client.clone(),
            edit,
        ))))
    }
}

// a bytes upload carries no name, so B512 names it image.png or mask.png
fn nameless(file: &ImageFile) -> ImageEditFile {
    ImageEditFile {
        content: file.content.clone(),
        name: None,
    }
}

struct OwnedEdit {
    redactor: Redactor,
    config: OpenAiConfig,
    model: String,
    prompt: Option<String>,
    optional_params: ImageParams,
    litellm_params: ImageParams,
    extra_headers: Option<ImageHeaders>,
    timeout: Duration,
    mode: ClientKind,
}

struct Pooled(ProxiedClient);

impl ClientSource for Pooled {
    type Client = ProxiedClient;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<ProxiedClient, ClientError> {
        Ok(self.0.clone())
    }
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// the handler's pre-call runs over a fresh logging state, as a request without callbacks would
fn logging_state(redactor: &Redactor) -> Result<LoggingState, ImageEditError> {
    let init: LoggingInit = serde_json::from_str(LOGGING_INIT)?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default,
    )
    .map_err(|error| ImageEditError::Invalid(error.to_string()))
}

fn pre_post_env(redactor: &Redactor) -> PrePostEnv<'_> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::UNIX_EPOCH.fixed_offset(),
    }
}

async fn send_edit(
    client: ProxiedClient,
    edit: OwnedEdit,
) -> Result<ImageResponse, ImageEditError> {
    let mut state = logging_state(&edit.redactor)?;
    let mut log = LogContext {
        state: &mut state,
        env: &pre_post_env(&edit.redactor),
    };
    let deps = ImageEditDeps {
        config: &edit.config,
        source: &Pooled(client),
        supplied: None,
    };
    let call = HandlerCall {
        model: &edit.model,
        prompt: edit.prompt.as_deref(),
        image: None,
        optional_params: &edit.optional_params,
        litellm_params: &edit.litellm_params,
        extra_headers: edit.extra_headers.as_ref(),
        timeout: edit.timeout,
    };
    let result = match edit.mode {
        ClientKind::Sync => image_edit_handler(&deps, &call, &mut log).await,
        ClientKind::Asynchronous => async_image_edit_handler(&deps, &call, &mut log).await,
    };
    match result {
        Ok(outcome) => Ok(outcome.response),
        Err(ImageEditHttpError::Provider(error)) => {
            Err(ImageEditError::Provider(ProviderFailure {
                status_code: error.status_code,
                message: error.message,
                status_code_is_synthesized: error.status_code_is_synthesized,
            }))
        }
        Err(other) => Err(ImageEditError::Transport {
            message: other.to_string(),
        }),
    }
}
