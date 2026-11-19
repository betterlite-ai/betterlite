use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::file_http_upload::config::{
    FileParams, FileResponse, FileUploadConfig, Headers, UploadParams,
};
use betterlite_core::api::file_http_upload::error::{TransformFailure, UploadError, UrlFailure};
use betterlite_core::api::file_http_upload::plan::{
    MediaPlan, MultipartPlan, PresignedPlan, TwoStepPlan, UploadPlan,
};
use betterlite_core::api::file_http_upload::service::{
    ClientSource, LogContext, Mode, UploadCall, UploadDeps, prepare, send,
};
use betterlite_core::api::vector_store_http_write::error::{BaseLlmException, ClientError};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::api::file_type_records::UploadData;
use betterlite_openai::files::records::{CreatePurpose, FileCreate, FileUpload};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::Method;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Number;
use tokio_util::task::TaskTracker;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::{
    Call, Content, Diagnostics, Failure, MultipartSpec, Outcome, Plan, PresignedSpec, RunMode,
    TwoStepSpec, UploadMethod,
};

const BOUNDARY: &str = "betterlite-oracle-boundary";
const PLACEHOLDER: &str = "{{upstream}}";
const ADDITIONAL_ARGS: &str = "additional_args";
const INIT: &str = r#"{"model":"file-upload","call_type":"create_file","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

// The production service over the mock's base: the case's plan stands in for the provider transform,
// and the case's headers for what validate_environment would return
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    _env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let origin = api_base.strip_suffix("/v1").unwrap_or(api_base).to_owned();
    let config = FixtureConfig {
        call,
        origin: origin.clone(),
    };
    let source = Source::new();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    let sink = Sink;
    let now = fixture_now()?;
    let env = fixture_env(&redactor, &sink, now);
    let mut state = logging_state(&redactor)?;
    let tasks = TaskTracker::new();
    let boundaries = || String::from(BOUNDARY);
    let params: FileParams = call.litellm_params.clone();
    let file = file_create(call);
    let upload_params = upload_params(api_base, &params);
    let upload_call = UploadCall {
        data: &file,
        params: &upload_params,
    };
    let deps = UploadDeps {
        config: &config,
        source: &source,
        supplied: None,
        tasks: &tasks,
        boundaries: &boundaries,
        limits: ApiLimits::default(),
    };
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let prepared = prepare(&deps, mode(call.mode), &upload_call, &mut log)
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let result = send(prepared).await;
    let diagnostics = match call.mode {
        RunMode::Sync => None,
        RunMode::Asynchronous => Some(diagnostics(&state, &origin)),
    };
    let answer = outcome(result, diagnostics, &origin)?;
    Ok(SdkResult::FileHttpUpload {
        value: Box::new(answer),
    })
}

struct FixtureConfig<'call> {
    call: &'call Call,
    origin: String,
}

impl FileUploadConfig for FixtureConfig<'_> {
    fn validate_environment(&self, _headers: Headers, _params: &UploadParams<'_>) -> Headers {
        base_headers(self.call)
    }

    fn get_complete_file_url(
        &self,
        params: &UploadParams<'_>,
        _data: &FileCreate,
    ) -> Result<Option<String>, UrlFailure> {
        Ok(params.api_base.map(ToOwned::to_owned))
    }

    fn file_upload_http_method(&self) -> Method {
        match self.call.upload_method {
            UploadMethod::Post => Method::POST,
            UploadMethod::Put => Method::PUT,
        }
    }

    fn transform_create_file_request(
        &self,
        _data: &FileCreate,
        _params: &UploadParams<'_>,
    ) -> Result<UploadPlan, TransformFailure> {
        Ok(upload_plan(&self.call.plan, &self.origin))
    }

    fn transform_create_file_response(
        &self,
        response: &FileResponse,
        _params: &FileParams,
    ) -> Result<Json, BaseLlmException> {
        Ok(file_object(response))
    }
}

struct Source {
    pool: ClientPool,
    tls: TlsPolicy,
    routing: Arc<ProxyPolicy>,
}

impl Source {
    fn new() -> Self {
        Self {
            pool: ClientPool::new(1, TransportLimits::DEFAULT),
            tls: TlsPolicy {
                verification: Verification::Enabled,
                ca_file: None,
                certificate: None,
                http2: false,
                ecdh_curve: None,
                user_agent: String::from("betterlite-oracle"),
            },
            routing: Arc::new(ProxyPolicy {
                no_proxy: String::new(),
                primary: BTreeMap::new(),
                fallback: BTreeMap::new(),
                warnings: Vec::new(),
            }),
        }
    }
}

impl ClientSource for Source {
    type Client = ProxiedClient;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<ProxiedClient, ClientError> {
        self.pool
            .client(&self.tls, &self.routing)
            .map_err(ClientError::Pool)
    }
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

fn fixture_now() -> Result<DateTime<FixedOffset>, SdkHttpError> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))
}

fn logging_state(redactor: &Redactor) -> Result<LoggingState, SdkHttpError> {
    let init: LoggingInit =
        serde_json::from_str(INIT).map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default(),
    )
    .map_err(SdkHttpError::LoggingState)
}

fn fixture_env<'env>(
    redactor: &'env Redactor,
    sink: &'env Sink,
    now: DateTime<FixedOffset>,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now,
    }
}

fn base_headers(call: &Call) -> Headers {
    call.headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
        .collect()
}

fn file_create(call: &Call) -> FileCreate {
    FileCreate {
        file: FileUpload {
            filename: call.file.filename.clone(),
            content_type: call.file.content_type.clone(),
            data: call.file.content.bytes(),
        },
        purpose: CreatePurpose::Batch,
        expires_after: None,
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: call.timeout_seconds.map(Duration::from_secs),
    }
}

fn upload_params<'params>(
    api_base: &'params str,
    params: &'params FileParams,
) -> UploadParams<'params> {
    UploadParams {
        api_base: Some(api_base),
        api_key: None,
        ssl_verify: None,
        litellm_params: params,
    }
}

const fn mode(mode: RunMode) -> Mode {
    match mode {
        RunMode::Sync => Mode::Sync,
        RunMode::Asynchronous => Mode::Asynchronous,
    }
}

fn upload_plan(spec: &Plan, origin: &str) -> UploadPlan {
    match spec {
        Plan::TwoStep(two_step) => UploadPlan::TwoStep(Box::new(two_step_plan(two_step, origin))),
        Plan::Presigned(presigned) => {
            UploadPlan::Presigned(Box::new(presigned_plan(presigned, origin)))
        }
        Plan::Media(media) => UploadPlan::Media(Box::new(MediaPlan {
            body_stream: Box::new(media.chunks.clone().into_iter()),
            content_type: media.content_type.clone(),
        })),
        Plan::Multipart(multipart) => UploadPlan::Multipart(Box::new(multipart_plan(multipart))),
        Plan::Text(text) => UploadPlan::Text(text.clone()),
        Plan::Raw(bytes) => UploadPlan::Bytes(bytes.clone()),
    }
}

fn two_step_plan(spec: &TwoStepSpec, origin: &str) -> TwoStepPlan {
    TwoStepPlan {
        initial_method: spec.initial_request.method.clone(),
        initial_url: spec
            .initial_request
            .url
            .as_deref()
            .map(|url| resolve(url, origin)),
        initial_headers: spec.initial_request.headers.clone(),
        initial_data: Json::Object(spec.initial_request.data.clone()),
        upload_method: spec.upload_request.method.clone(),
        upload_url: spec
            .upload_request
            .url
            .as_deref()
            .map(|url| resolve(url, origin)),
        upload_headers: spec.upload_request.headers.clone(),
        upload_data: upload_data(&spec.upload_request.data),
        url_location: spec.upload_url_location,
        url_key: spec.upload_url_key.clone(),
    }
}

fn presigned_plan(spec: &PresignedSpec, origin: &str) -> PresignedPlan {
    PresignedPlan {
        method: spec.method.clone(),
        url: resolve(&spec.url, origin),
        headers: spec.headers.clone(),
        data: upload_data(&spec.data),
    }
}

fn multipart_plan(spec: &MultipartSpec) -> MultipartPlan {
    MultipartPlan {
        parts: spec
            .parts
            .iter()
            .map(|part| MultipartPart {
                name: part.name.clone(),
                file_name: part.filename.clone(),
                content_type: part.content_type.clone(),
                data: part.content.bytes(),
            })
            .collect(),
    }
}

fn upload_data(content: &Content) -> UploadData {
    match content {
        Content::Text(text) => UploadData::Text(text.clone()),
        Content::Raw(bytes) => UploadData::Bytes(bytes.clone()),
    }
}

fn resolve(url: &str, origin: &str) -> String {
    url.replace(PLACEHOLDER, origin)
}

fn file_object(response: &FileResponse) -> Json {
    let mut object = IndexMap::new();
    object.insert(String::from("id"), Json::Text(String::from("file-oracle")));
    object.insert(String::from("object"), Json::Text(String::from("file")));
    object.insert(
        String::from("bytes"),
        Json::Number(Number::from(response.body.len())),
    );
    object.insert(
        String::from("created_at"),
        Json::Number(Number::from(0_u32)),
    );
    object.insert(
        String::from("filename"),
        Json::Text(String::from("oracle.txt")),
    );
    object.insert(String::from("purpose"), Json::Text(String::from("batch")));
    Json::Object(object)
}

fn diagnostics(state: &LoggingState, origin: &str) -> Diagnostics {
    let details = &state.model_call_details;
    Diagnostics {
        input: field(details, "input", origin),
        api_key: field(details, "api_key", origin),
        api_base: argument(details, "api_base", origin),
        headers: argument(details, "headers", origin),
        complete_input_dict: argument(details, "complete_input_dict", origin),
    }
}

fn field(details: &IndexMap<String, Json>, name: &str, origin: &str) -> Json {
    let value = details.get(name).cloned().unwrap_or(Json::Null);
    unalias(value, origin)
}

fn argument(details: &IndexMap<String, Json>, name: &str, origin: &str) -> Json {
    let Some(Json::Object(args)) = details.get(ADDITIONAL_ARGS) else {
        return Json::Null;
    };
    let value = args.get(name).cloned().unwrap_or(Json::Null);
    unalias(value, origin)
}

fn outcome(
    result: Result<Json, UploadError>,
    diagnostics: Option<Diagnostics>,
    origin: &str,
) -> Result<Outcome, SdkHttpError> {
    match result {
        Ok(value) => Ok(Outcome {
            result: Some(unalias(value, origin)),
            failure: None,
            diagnostics,
        }),
        Err(UploadError::Provider(error)) => Ok(Outcome {
            result: None,
            failure: Some(failure(error, origin)),
            diagnostics,
        }),
        Err(other) => Err(SdkHttpError::Invalid(other.to_string())),
    }
}

fn failure(error: BaseLlmException, origin: &str) -> Failure {
    Failure {
        status: error.status_code,
        synthesized: error.status_code_is_synthesized,
        message: error.message,
        headers: error
            .headers
            .into_iter()
            .map(|(name, value)| (name, unalias(value, origin)))
            .collect(),
    }
}

// the mock's origin differs per side, so every observed string spells it as the placeholder
fn unalias(value: Json, origin: &str) -> Json {
    match value {
        Json::Text(text) => Json::Text(text.replace(origin, PLACEHOLDER)),
        Json::List(items) => Json::List(
            items
                .into_iter()
                .map(|item| unalias(item, origin))
                .collect(),
        ),
        Json::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(name, value)| (name, unalias(value, origin)))
                .collect(),
        ),
        Json::Null => Json::Null,
        Json::Bool(flag) => Json::Bool(flag),
        Json::Number(number) => Json::Number(number),
    }
}
