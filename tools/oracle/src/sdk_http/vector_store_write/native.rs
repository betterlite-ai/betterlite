use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_config::api::vector_store_write::loads_request_timeout;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_core::api::vector_store_http_write::config::{OpenAiWriteConfig, WriteParams};
use betterlite_core::api::vector_store_http_write::error::WriteError;
use betterlite_core::api::vector_store_http_write::service::{
    DeleteCall, LogContext, UpdateCall, WriteCall as HttpWriteCall, WriteDeps, WriteTarget,
    prepare, send,
};
use betterlite_core::api::vector_store_write::error::VectorStoreWriteFailure;
use betterlite_core::api::vector_store_write::service::{
    Operation as CoreOperation, WriteCall as CoreWriteCall, WriteService,
};
use betterlite_core::api::vector_store_write::{
    DeleteRequest, UpdateRequest, VectorStoreWriteDeps, adelete, aupdate, delete, update,
};
use betterlite_error::envelope::WireField;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::super::result::{Failure, Nullable};
use super::support::{Resolution, Sink, Source, logging_state, moment, source};
use super::{Call, Mode, Operation, Outcome, Run, RunOutcome};

// The production service over the mock's base: the same config sections and pooled client a request would build
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let config = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let request_timeout_seconds =
        loads_request_timeout(env).map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let service = write_service(&config, api_base, env)?;
    let resolution = Resolution::new(env);
    let context = resolution.context();
    let mut runs = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        runs.push(execute_run(run, &service, &context, request_timeout_seconds).await?);
    }
    Ok(SdkResult::VectorStoreWrite {
        value: Box::new(Outcome { runs }),
    })
}

fn write_service<'config>(
    config: &'config Config,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<Service<'config>, SdkHttpError> {
    Ok(Service {
        config: OpenAiWriteConfig::new(
            &config.sections.openai_credentials,
            LiteLlmGlobals::default(),
            config.sections.resource_sdk_settings.openai_api_key.clone(),
            None,
        ),
        source: source(config, env)?,
        params: WriteParams {
            api_base: Some(api_base.to_owned()),
            api_key: None,
            ssl_verify: None,
        },
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?,
        sink: Sink,
        now: moment()?,
    })
}

async fn execute_run(
    run: &Run,
    service: &Service<'_>,
    context: &ResolutionContext<'_>,
    request_timeout_seconds: f64,
) -> Result<RunOutcome, SdkHttpError> {
    let call_type = match run.operation {
        Operation::Update => "update",
        Operation::Delete => "delete",
    };
    let logging = Mutex::new(logging_state(call_type)?);
    let deps = VectorStoreWriteDeps {
        logging: &logging,
        redactor: &service.redactor,
        resolution: context,
        request_timeout_seconds,
    };
    let kwargs = run.kwargs();
    let written = match run.operation {
        Operation::Update => update_run(run, service, deps, &kwargs).await,
        Operation::Delete => delete_run(run, service, deps, &kwargs).await,
    };
    Ok(match written {
        Ok(value) => RunOutcome {
            value: Some(value),
            failure: None,
        },
        Err(failure) => RunOutcome {
            value: None,
            failure: Some(project(&failure)?),
        },
    })
}

async fn update_run(
    run: &Run,
    service: &Service<'_>,
    deps: VectorStoreWriteDeps<'_>,
    kwargs: &IndexMap<String, Json>,
) -> Result<Json, VectorStoreWriteFailure> {
    let request = UpdateRequest {
        vector_store_id: &run.vector_store_id,
        name: run.name.as_deref(),
        expires_after: run.expires_after.as_ref(),
        metadata: run.metadata.as_ref(),
        extra_headers: run.extra_headers.as_ref(),
        extra_query: run.extra_query.as_ref(),
        extra_body: run.extra_body.as_ref(),
        timeout: run.timeout,
        custom_llm_provider: run.custom_llm_provider.as_deref(),
        params: &service.params,
        kwargs,
    };
    match run.mode {
        Mode::Sync => update(service, deps, &request).await,
        Mode::Asynchronous => aupdate(service, deps, &request).await,
    }
}

async fn delete_run(
    run: &Run,
    service: &Service<'_>,
    deps: VectorStoreWriteDeps<'_>,
    kwargs: &IndexMap<String, Json>,
) -> Result<Json, VectorStoreWriteFailure> {
    let request = DeleteRequest {
        vector_store_id: &run.vector_store_id,
        extra_headers: run.extra_headers.as_ref(),
        extra_query: run.extra_query.as_ref(),
        extra_body: run.extra_body.as_ref(),
        timeout: run.timeout,
        custom_llm_provider: run.custom_llm_provider.as_deref(),
        params: &service.params,
        kwargs,
    };
    match run.mode {
        Mode::Sync => delete(service, deps, &request).await,
        Mode::Asynchronous => adelete(service, deps, &request).await,
    }
}

// The exception the public function raises, as `result::Failure` renders it for both sides
fn project(failure: &VectorStoreWriteFailure) -> Result<Failure, SdkHttpError> {
    let raised = failure
        .raised()
        .map_err(|error| SdkHttpError::Unprojected(error.to_string()))?;
    Ok(Failure {
        class: raised.class.to_owned(),
        status: raised.status,
        kind: field(raised.kind),
        param: field(raised.param),
        code: Some(raised.code),
        message: raised.message,
    })
}

// None: key absent, Some(None): JSON null, Some(Some(text)): text
fn field(wire: WireField) -> Nullable {
    match wire {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}

// The write service B2402 runs in production, with the logging state the call built and B247's environment
struct Service<'config> {
    config: OpenAiWriteConfig<'config>,
    source: Source,
    params: WriteParams,
    redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
}

impl Service<'_> {
    fn pre_post(&self) -> PrePostEnv<'_> {
        PrePostEnv {
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
        }
    }
}

impl WriteService for Service<'_> {
    async fn write(&self, call: &CoreWriteCall<'_>) -> Result<Json, WriteError> {
        let deps = WriteDeps {
            config: &self.config,
            source: &self.source,
            supplied: None,
        };
        let target = WriteTarget {
            vector_store_id: call.vector_store_id,
            params: &self.params,
            extra_headers: call.extra_headers,
            timeout: Some(call.timeout),
        };
        let write = match call.operation {
            CoreOperation::Update => HttpWriteCall::Update(UpdateCall {
                target,
                optional_params: call.optional_params,
                extra_body: call.extra_body,
            }),
            CoreOperation::Delete => HttpWriteCall::Delete(DeleteCall {
                target,
                extra_body: call.extra_body,
            }),
        };
        let env = self.pre_post();
        let prepared = {
            let mut state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
            prepare(
                &deps,
                call.mode,
                &write,
                &mut LogContext {
                    state: &mut state,
                    env: &env,
                },
            )?
        };
        send(prepared).await
    }
}
