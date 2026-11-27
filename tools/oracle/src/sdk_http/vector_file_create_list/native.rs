use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_config::api::vector_store_write::loads_request_timeout;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_core::api::vector_file_create::service::{
    FileCall, FileService, Operation as CoreOperation,
};
use betterlite_core::api::vector_file_create::status::StatusCheckedFilesConfig;
use betterlite_core::api::vector_file_create::{
    CreateFileRequest, ListFileRequest, TransportRequest, VectorFileCreateDeps, VectorFileCreates,
};
use betterlite_core::api::vector_file_http_create_list::config::OpenAiFilesConfig;
use betterlite_core::api::vector_file_http_create_list::error::FileTransportError;
use betterlite_core::api::vector_file_http_create_list::service::{
    CreateCall, FileCall as HttpFileCall, FileDeps, FileTarget, ListCall, LogContext, prepare, send,
};
use betterlite_core::api::vector_file_provider::CredentialLookup;
use betterlite_core::api::vector_store_search::error::SdkFailure;
use betterlite_error::envelope::WireField;
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use indexmap::IndexMap;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::super::result::{Failure, Nullable};
use super::support::{Assets, CaseRegistry, Source, logging_state, registry_of, source};
use super::{Call, CreateArgs, ListArgs, Mode, Operation, Outcome, Run, RunOutcome};

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
    let assets = Assets::new()?;
    let origin = source(&config, env)?;
    let registry = call
        .registry
        .as_ref()
        .map(|registry| registry_of(registry, api_base));
    let service = file_service(&config, &origin, &assets);
    let mut runs = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        runs.push(
            run_once(
                run,
                api_base,
                &service,
                registry.as_ref(),
                request_timeout_seconds,
            )
            .await?,
        );
    }
    Ok(SdkResult::VectorFileCreateList {
        value: Box::new(Outcome { runs }),
    })
}
fn file_service<'config, 'assets>(
    config: &'config Config,
    origin: &'assets Source,
    assets: &'assets Assets,
) -> Service<'assets, 'config> {
    let provider = OpenAiFilesConfig::new(
        &config.sections.openai_credentials,
        LiteLlmGlobals::default(),
        config.sections.resource_sdk_settings.openai_api_key.clone(),
        None,
    );
    Service {
        config: StatusCheckedFilesConfig::new(provider),
        source: origin,
        assets,
    }
}

async fn run_once(
    run: &Run,
    api_base: &str,
    service: &Service<'_, '_>,
    registry: Option<&CaseRegistry>,
    request_timeout_seconds: f64,
) -> Result<RunOutcome, SdkHttpError> {
    let logging = Mutex::new(logging_state(call_type(run.operation))?);
    let lookup = registry.map(|found: &CaseRegistry| -> &dyn CredentialLookup { found });
    let deps = VectorFileCreateDeps {
        logging: &logging,
        redactor: &service.assets.redactor,
        registry: lookup,
        request_timeout_seconds,
    };
    let kwargs = kwargs_of(run, api_base);
    let answer = match run.operation {
        Operation::Create => {
            let args = run.create.as_ref().ok_or_else(|| missing("create"))?;
            create_run(run, args, service, deps, &kwargs).await
        }
        Operation::List => {
            let args = run.list.as_ref().ok_or_else(|| missing("list"))?;
            list_run(run, args, service, deps, &kwargs).await
        }
    };
    let failure = answer.as_ref().err().map(project);
    Ok(RunOutcome {
        operation: run.operation,
        mode: run.mode,
        result: answer.ok(),
        failure,
    })
}
const fn call_type(operation: Operation) -> &'static str {
    match operation {
        Operation::Create => "vector_store_file_create",
        Operation::List => "vector_store_file_list",
    }
}

async fn create_run(
    run: &Run,
    args: &CreateArgs,
    service: &Service<'_, '_>,
    deps: VectorFileCreateDeps<'_>,
    kwargs: &IndexMap<String, Json>,
) -> Result<Json, SdkFailure> {
    let request = CreateFileRequest {
        transport: transport(run, kwargs),
        file_id: &args.file_id,
        attributes: args.attributes.as_ref(),
        chunking_strategy: args.chunking_strategy.as_ref(),
    };
    let files = VectorFileCreates { service, deps };
    match run.mode {
        Mode::Sync => files.create(&request).await,
        Mode::Asynchronous => files.acreate(&request).await,
    }
}

async fn list_run(
    run: &Run,
    args: &ListArgs,
    service: &Service<'_, '_>,
    deps: VectorFileCreateDeps<'_>,
    kwargs: &IndexMap<String, Json>,
) -> Result<Json, SdkFailure> {
    let request = ListFileRequest {
        transport: transport(run, kwargs),
        after: args.after.as_deref(),
        before: args.before.as_deref(),
        filter: args.filter.as_deref(),
        limit: args.limit,
        order: args.order.as_deref(),
    };
    let files = VectorFileCreates { service, deps };
    match run.mode {
        Mode::Sync => files.list(&request).await,
        Mode::Asynchronous => files.alist(&request).await,
    }
}
fn transport<'run>(run: &'run Run, kwargs: &'run IndexMap<String, Json>) -> TransportRequest<'run> {
    TransportRequest {
        vector_store_id: &run.vector_store_id,
        extra_headers: run.extra_headers.as_ref(),
        extra_query: run.extra_query.as_ref(),
        extra_body: run.extra_body.as_ref(),
        timeout: run.timeout_seconds,
        custom_llm_provider: run.custom_llm_provider.as_deref(),
        kwargs,
    }
}

// GenericLiteLLMParams(**kwargs) plus the base the mock answered on
fn kwargs_of(run: &Run, api_base: &str) -> IndexMap<String, Json> {
    let mut kwargs = IndexMap::from([(String::from("api_base"), Json::Text(api_base.to_owned()))]);
    if let Some(key) = &run.options.api_key {
        kwargs.insert(String::from("api_key"), Json::Text(key.clone()));
    }
    kwargs.insert(
        String::from("litellm_call_id"),
        Json::Text(run.options.litellm_call_id.clone()),
    );
    if let Some(unknown) = &run.unknown {
        kwargs.insert(String::from("unknown"), unknown.clone());
    }
    kwargs
}
fn missing(operation: &str) -> SdkHttpError {
    SdkHttpError::Invalid(format!(
        "a {operation} run carries the {operation} arguments"
    ))
}
fn project(failure: &SdkFailure) -> Failure {
    Failure {
        class: failure.class.clone(),
        status: failure.status,
        kind: field(failure.kind.clone()),
        param: field(failure.param.clone()),
        code: field(failure.code.clone()),
        message: failure.message.clone(),
    }
}

const fn field(wire: WireField) -> Nullable {
    match wire {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}

// The production service over B2480's handlers, with the status check LiteLLM's httpx client adds
impl FileService for Service<'_, '_> {
    async fn file(&self, call: &FileCall<'_>) -> Result<Json, FileTransportError> {
        let deps = FileDeps {
            config: &self.config,
            source: self.source,
            supplied: None,
        };
        let target = FileTarget {
            vector_store_id: call.vector_store_id,
            params: call.params,
            extra_headers: call.extra_headers,
            timeout: Some(call.timeout),
        };
        let transport = match call.operation {
            CoreOperation::Create => HttpFileCall::Create(CreateCall {
                target,
                create_request: call.fields,
                extra_body: call.extra_body,
            }),
            CoreOperation::List => HttpFileCall::List(ListCall {
                target,
                query_params: call.fields,
                extra_query: call.extra_query,
            }),
        };
        let env = self.assets.env();
        let prepared = {
            let mut state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
            prepare(
                &deps,
                call.mode,
                &transport,
                &mut LogContext {
                    state: &mut state,
                    env: &env,
                },
            )?
        };
        send(prepared).await
    }
}

struct Service<'assets, 'config> {
    config: StatusCheckedFilesConfig<OpenAiFilesConfig<'config>>,
    source: &'assets Source,
    assets: &'assets Assets,
}
