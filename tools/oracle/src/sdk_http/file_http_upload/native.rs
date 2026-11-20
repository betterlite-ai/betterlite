use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::file_http_upload::config::{
    FileParams, FileResponse, FileUploadConfig, Headers, UploadParams,
};
use betterlite_core::api::file_http_upload::error::{TransformFailure, UrlFailure};
use betterlite_core::api::file_http_upload::plan::UploadPlan;
use betterlite_core::api::file_http_upload::service::{
    LogContext, Mode, UploadCall, UploadDeps, prepare, send,
};
use betterlite_core::api::vector_store_http_write::error::BaseLlmException;
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::{CreatePurpose, FileCreate, FileUpload};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::Method;
use indexmap::IndexMap;
use tokio_util::task::TaskTracker;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::{Call, RunMode, UploadMethod};

mod fixture;
mod observe;
mod plan;

use fixture::{Sink, Source, base_headers, fixture_env, fixture_now, logging_state};
use observe::{diagnostics, file_object, outcome};
use plan::upload_plan;

const BOUNDARY: &str = "betterlite-oracle-boundary";
const PLACEHOLDER: &str = "{{upstream}}";
const ADDITIONAL_ARGS: &str = "additional_args";
const INIT: &str = r#"{"model":"file-upload","call_type":"create_file","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

// The production service over the mock's base, with the case's plan standing in for the transform
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
