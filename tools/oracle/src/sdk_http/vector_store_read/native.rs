use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Mutex, PoisonError};

use betterlite_config::api::vector_store_read::loads_read_timeout;
use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::vector_store_http_create_read::error::CreateReadError;
use betterlite_core::api::vector_store_http_create_read::service::{
    CreateReadCall, CreateReadDeps, CreateReadTarget, ListCall as HttpListCall,
    ListQuery as HttpListQuery, RetrieveCall, prepare, send,
};
use betterlite_core::api::vector_store_http_write::error::{
    BaseLlmException, HeaderMap, WriteError,
};
use betterlite_core::api::vector_store_http_write::service::{LogContext, SuppliedClient};
use betterlite_core::api::vector_store_read::error::VectorStoreReadFailure;
use betterlite_core::api::vector_store_read::service::{
    Operation as ReadOperation, ReadCall, ReadProvider,
};
use betterlite_core::api::vector_store_read::{
    ReadResult, TransportRequest, VectorStoreListRequest, VectorStoreReadDeps,
    VectorStoreReadRequest, VectorStoreReads,
};
use betterlite_error::envelope::WireField;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::super::result::{Failure, Nullable};
use super::support::{Assets, Configs, Source, Transport, source};
use super::{Call, Context, Mode, Operation, Outcome, Run, RunOutcome};

// The production service over the mock's base: the env's config, the pooled client and the read core a request would run
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
    let fixtures = Fixtures {
        configs: Configs::new(&config),
        source: source(&config, env)?,
        assets: Assets::new()?,
        request_timeout: loads_read_timeout(env)
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?,
    };
    let mut runs = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        runs.push(run_once(run, api_base, &fixtures).await?);
    }
    Ok(SdkResult::VectorStoreRead {
        value: Box::new(Outcome { runs }),
    })
}

// The parts every run lends to the core service, built once for the whole call
struct Fixtures {
    configs: Configs,
    source: Source,
    assets: Assets,
    request_timeout: f64,
}

async fn run_once(
    run: &Run,
    api_base: &str,
    fixtures: &Fixtures,
) -> Result<RunOutcome, SdkHttpError> {
    let logging = Mutex::new(fixtures.assets.state(call_type(run.operation))?);
    let transport = Transport::new(run, api_base);
    let deps = VectorStoreReadDeps {
        configs: &fixtures.configs,
        logging: &logging,
        redactor: &fixtures.assets.redactor,
        supplied: None,
        request_timeout_seconds: fixtures.request_timeout,
    };
    let provider = Native {
        source: &fixtures.source,
        assets: &fixtures.assets,
    };
    let reads = VectorStoreReads {
        provider: &provider,
        deps,
    };
    let answer = match run.operation {
        Operation::Retrieve => {
            let vector_store_id = retrieve_id(run)?;
            dispatch_retrieve(&reads, run.mode, transport.request(run), vector_store_id).await
        }
        Operation::List => dispatch_list(&reads, run, transport.request(run)).await,
    };
    let failure = answer.as_ref().err().map(project);
    let state = logging.lock().unwrap_or_else(PoisonError::into_inner);
    let context = context_of(&state);
    Ok(RunOutcome {
        operation: run.operation,
        mode: run.mode,
        result: answer.ok(),
        failure,
        context,
    })
}

const fn call_type(operation: Operation) -> &'static str {
    match operation {
        Operation::Retrieve => "vector_store_retrieve",
        Operation::List => "vector_store_list",
    }
}

// retrieve's HTTP read the way its handler picks it: the explicit aretrieve, otherwise the kwargs flag
async fn dispatch_retrieve(
    reads: &VectorStoreReads<'_, Native<'_>>,
    mode: Mode,
    transport: TransportRequest<'_>,
    vector_store_id: &str,
) -> ReadResult {
    let request = VectorStoreReadRequest {
        transport,
        vector_store_id,
    };
    match mode {
        Mode::Sync => reads.retrieve(&request).await,
        Mode::Asynchronous => reads.aretrieve(&request).await,
    }
}

// list's HTTP read the same way, over the four cursor arguments the case spelled
async fn dispatch_list(
    reads: &VectorStoreReads<'_, Native<'_>>,
    run: &Run,
    transport: TransportRequest<'_>,
) -> ReadResult {
    let request = VectorStoreListRequest {
        transport,
        after: run.after.as_deref(),
        before: run.before.as_deref(),
        limit: run.limit,
        order: run.order.as_ref().map(|order| order.as_deref()),
    };
    if matches!(run.mode, Mode::Asynchronous) {
        return reads.alist(&request).await;
    }
    reads.list(&request).await
}

fn retrieve_id(run: &Run) -> Result<&str, SdkHttpError> {
    run.vector_store_id
        .as_deref()
        .ok_or_else(|| SdkHttpError::Invalid("a retrieve run carries a vector_store_id".to_owned()))
}

// update_from_kwargs is what names the provider; a call that failed before it has no context to record
fn context_of(state: &LoggingState) -> Option<Context> {
    let provider = state.custom_llm_provider.as_ref()?;
    Some(Context {
        model: state.model.clone(),
        optional_params: state
            .optional_params
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        custom_llm_provider: provider.clone(),
        litellm_call_id: state
            .litellm_params
            .get("litellm_call_id")
            .cloned()
            .unwrap_or(Json::Null),
    })
}

// The exception a caller observes, with the wire fields exception_type's class carries
fn project(failure: &VectorStoreReadFailure) -> Failure {
    let raised = failure.raised();
    Failure {
        class: raised.class.to_owned(),
        status: raised.status,
        kind: field(raised.kind),
        param: field(raised.param),
        code: Some(raised.code),
        message: raised.message,
    }
}

// mirrors getattr(e, field, "None"): absent, present and null, or text
fn field(wire: WireField) -> Nullable {
    match wire {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}

// The read core behind B2485: the same prepare and send the production adapter would run
struct Native<'fixtures> {
    source: &'fixtures Source,
    assets: &'fixtures Assets,
}

impl ReadProvider for Native<'_> {
    type Client = ProxiedClient;

    fn read(
        &self,
        call: &ReadCall<'_, ProxiedClient>,
    ) -> impl Future<Output = Result<Json, CreateReadError>> + Send {
        async move {
            let deps = CreateReadDeps {
                config: &call.config,
                source: self.source,
                supplied: call.client.map(|supplied| SuppliedClient {
                    kind: supplied.kind,
                    transport: supplied.transport,
                }),
            };
            let http_call = http_call_of(call)?;
            let env = self.assets.env();
            let prepared = {
                let mut state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
                prepare(
                    &deps,
                    call.mode,
                    &http_call,
                    &mut LogContext {
                        state: &mut state,
                        env: &env,
                    },
                )
            };
            send(prepared?).await
        }
    }
}

// B2485's HTTP call for a read: retrieve's id, or list's four cursor fields over the same target
fn http_call_of<'call>(
    call: &ReadCall<'call, ProxiedClient>,
) -> Result<CreateReadCall<'call>, CreateReadError> {
    let target = CreateReadTarget {
        params: call.params,
        extra_headers: call.extra_headers,
        timeout: Some(call.timeout),
    };
    match call.operation {
        ReadOperation::Retrieve => {
            let Some(vector_store_id) = call.vector_store_id else {
                // the read core only builds a retrieve with its id, so this guards a broken caller
                return Err(missing_id());
            };
            Ok(CreateReadCall::Retrieve(RetrieveCall {
                target,
                vector_store_id,
                extra_body: call.extra_body,
            }))
        }
        ReadOperation::List => Ok(CreateReadCall::List(HttpListCall {
            target,
            query: HttpListQuery {
                after: call.query.after,
                before: call.query.before,
                limit: call.query.limit,
                order: call.query.order,
            },
            extra_body: call.extra_body,
        })),
    }
}

fn missing_id() -> CreateReadError {
    CreateReadError::Write(WriteError::Provider(BaseLlmException {
        status_code: 500,
        message: String::from("a retrieve read needs a vector store id"),
        headers: HeaderMap::new(),
        status_code_is_synthesized: true,
    }))
}
