use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_config::api::vector_file_write::loads_write_timeout;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_core::api::vector_file_http_write::config::OpenAiFileWriteConfig;
use betterlite_core::api::vector_file_http_write::error::FileWriteError;
use betterlite_core::api::vector_file_http_write::service::{
    DeleteCall, LogContext, UpdateCall, WriteCall as HttpWriteCall, WriteDeps, WriteTarget,
    prepare, send,
};
use betterlite_core::api::vector_file_provider::{
    CredentialLookup, CredentialLookupError, OperationKwargs,
};
use betterlite_core::api::vector_file_write::error::VectorFileWriteFailure;
use betterlite_core::api::vector_file_write::service::{WriteCall as CoreWriteCall, WriteService};
use betterlite_core::api::vector_file_write::{
    FileUpdateRequest, FileWriteRequest, VectorFileWriteDeps, VectorFileWrites,
};
use betterlite_core::api::vector_store_http_write::service::Operation as CoreOperation;
use betterlite_error::envelope::WireField;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::super::result::{Failure, Nullable};
use super::support::{Sink, Source, logging_state, moment, source};
use super::{Call, Mode, Operation, Outcome, Registry, Run, RunOutcome};

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
        loads_write_timeout(env).map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let service = write_service(&config, env)?;
    let mut runs = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        runs.push(execute_run(run, &service, api_base, request_timeout_seconds).await?);
    }
    Ok(SdkResult::VectorFileWrite {
        value: Box::new(Outcome { runs }),
    })
}

fn write_service<'config>(
    config: &'config Config,
    env: &BTreeMap<String, String>,
) -> Result<Service<'config>, SdkHttpError> {
    Ok(Service {
        config: OpenAiFileWriteConfig {
            settings: &config.sections.openai_credentials,
            globals: LiteLlmGlobals::default(),
            env_api_key: config.sections.resource_sdk_settings.openai_api_key.clone(),
            litellm_api_base: None,
        },
        source: source(config, env)?,
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?,
        sink: Sink,
        now: moment()?,
    })
}

async fn execute_run(
    run: &Run,
    service: &Service<'_>,
    api_base: &str,
    request_timeout_seconds: f64,
) -> Result<RunOutcome, SdkHttpError> {
    let call_type = match run.operation {
        Operation::Update => "update",
        Operation::Delete => "delete",
    };
    let logging = Mutex::new(logging_state(call_type)?);
    // a case without a registry behaves as an installed one that answers with no credentials
    let registry = run.registry.clone().unwrap_or_default();
    let deps = VectorFileWriteDeps {
        logging: &logging,
        registry: Some(&registry),
        redactor: &service.redactor,
        request_timeout_seconds,
    };
    let kwargs = kwargs_for(run, api_base);
    let written = write_run(run, service, deps, &kwargs).await;
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

// **kwargs as the public function takes them: the harness base, the caller's key, and what only the kwargs route carries
fn kwargs_for(run: &Run, api_base: &str) -> IndexMap<String, Json> {
    let mut kwargs = IndexMap::new();
    kwargs.insert(String::from("api_base"), Json::Text(api_base.to_owned()));
    if let Some(api_key) = &run.api_key {
        kwargs.insert(String::from("api_key"), Json::Text(api_key.clone()));
    }
    if let Some(query) = &run.extra_query {
        kwargs.insert(String::from("extra_query"), Json::Object(query.clone()));
    }
    // delete has no extra_body argument, so the value reaches LiteLLM through **kwargs and never the wire
    if run.operation == Operation::Delete
        && let Some(body) = &run.extra_body
    {
        kwargs.insert(String::from("extra_body"), Json::Object(body.clone()));
    }
    if let Some(unknown) = &run.unknown {
        kwargs.insert(String::from("unknown"), unknown.clone());
    }
    kwargs
}

async fn write_run(
    run: &Run,
    service: &Service<'_>,
    deps: VectorFileWriteDeps<'_>,
    kwargs: &IndexMap<String, Json>,
) -> Result<Json, VectorFileWriteFailure> {
    let write = FileWriteRequest {
        vector_store_id: &run.vector_store_id,
        file_id: &run.file_id,
        extra_headers: run.extra_headers.as_ref(),
        timeout: run.timeout,
        custom_llm_provider: run.custom_llm_provider.as_deref(),
        kwargs,
    };
    let update = FileUpdateRequest {
        write,
        attributes: run.attributes.as_ref(),
        extra_body: run.extra_body.as_ref(),
    };
    let writes = VectorFileWrites { service, deps };
    match (run.operation, run.mode) {
        (Operation::Update, Mode::Sync) => writes.update(&update).await,
        (Operation::Update, Mode::Asynchronous) => writes.aupdate(&update).await,
        (Operation::Delete, Mode::Sync) => writes.delete(&write).await,
        (Operation::Delete, Mode::Asynchronous) => writes.adelete(&write).await,
    }
}

// The exception the public function raises, as `result::Failure` renders it for both sides
fn project(failure: &VectorFileWriteFailure) -> Result<Failure, SdkHttpError> {
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

// The write service B2402 runs in production, with the logging state the call built and its environment
struct Service<'config> {
    config: OpenAiFileWriteConfig<'config>,
    source: Source,
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
    async fn write(&self, call: &CoreWriteCall<'_>) -> Result<Json, FileWriteError> {
        let deps = WriteDeps {
            config: &self.config,
            source: &self.source,
            supplied: None,
        };
        let target = WriteTarget {
            vector_store_id: call.vector_store_id,
            file_id: call.file_id,
            params: call.params,
            extra_headers: call.extra_headers,
            timeout: Some(call.timeout),
        };
        let write = match call.operation {
            CoreOperation::Update => HttpWriteCall::Update(UpdateCall {
                target,
                update_request: call.update_request,
                extra_body: call.extra_body,
            }),
            CoreOperation::Delete => HttpWriteCall::Delete(DeleteCall { target }),
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

// The case's registry: the credentials it overlays, or a lookup that raises and LiteLLM swallows
impl CredentialLookup for Registry {
    fn lookup_credentials(
        &self,
        _vector_store_id: &str,
    ) -> Result<OperationKwargs, CredentialLookupError> {
        if self.failing {
            return Err(CredentialLookupError::Unavailable {
                reason: String::from("the case registry raised"),
            });
        }
        Ok(OperationKwargs(self.credentials.clone()))
    }
}
