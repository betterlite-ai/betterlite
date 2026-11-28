use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::bedrock_embedding_io::credentials::ManagedRegionWorker;
use betterlite_bedrock::bedrock_embedding_io::transport::Timeout;
use betterlite_bedrock::embedding_call::error::EmbeddingCallError;
use betterlite_bedrock::embedding_call::plan::remaining_options;
use betterlite_bedrock::embedding_call::{
    Clock, Dependencies, EmbeddingCall, EmbeddingCallResponse, EmbeddingEnvironment, LogContext,
    ParamMap, embeddings,
};
use betterlite_bedrock::signing::executor::AwsSigningExecutor;
use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json as Decisions;
use betterlite_prompts::arguments::json::Object;
use betterlite_redact::secret::Redactor;
use secrecy::SecretString;
use serde_json::value::RawValue;
use tokio::runtime::{Builder, Runtime};

use super::super::input::{Case, Supplied};
use super::super::output::AdapterError;
use super::client::{Factory, FixtureClient, Sent, supplied_client};
use super::logging::{Capture, DiscardingSink, env, redactor, state};
use super::source::{Counting, Source, secret_lookup};

pub(crate) struct Harness {
    pub(crate) outcome: Result<EmbeddingCallResponse, EmbeddingCallError>,
    pub(crate) requests: Vec<Sent>,
    pub(crate) factory: Vec<serde_json::Value>,
    pub(crate) phases: Vec<String>,
    pub(crate) remaining: Object,
}

struct Execution<'run> {
    factory: &'run Factory,
    transport: &'run FixtureClient,
    supplied: &'run Option<(ClientKind, FixtureClient)>,
}

struct Logging {
    state: LoggingState,
    redactor: Redactor,
    sink: DiscardingSink,
    capture: Capture,
}

impl Logging {
    fn new() -> Result<Self, AdapterError> {
        let redactor = redactor()?;
        Ok(Self {
            state: state(&redactor)?,
            redactor,
            sink: DiscardingSink,
            capture: Capture::default(),
        })
    }
}

pub(crate) fn run(case: &Case) -> Result<Harness, AdapterError> {
    let factory = Factory::new(&case.replies);
    let transport = supplied_client(&factory);
    let supplied = supplied(case, &factory);
    let api_key = case.api_key.as_deref().map(SecretString::from);
    let bearer = bearer(case);
    let extra = case.extra_headers.as_ref().map(header_pairs);
    let mut call = embedding_call(case, api_key.as_ref(), bearer.as_ref(), extra.as_deref())?;
    let execution = Execution {
        factory: &factory,
        transport: &transport,
        supplied: &supplied,
    };
    let mut logging = Logging::new()?;
    let outcome = execute(case, &mut call, &execution, &mut logging)?;
    let mut requests = factory.requests();
    requests.extend(supplied.iter().flat_map(|(_, client)| client.requests()));
    requests.extend(transport.requests());
    Ok(Harness {
        outcome,
        requests,
        factory: factory.calls(),
        phases: logging.capture.phases(),
        remaining: remaining_options(&call.optional_params).map_err(|error| {
            AdapterError::Setup {
                message: error.to_string(),
            }
        })?,
    })
}

fn execute(
    case: &Case,
    call: &mut EmbeddingCall<'_>,
    execution: &Execution<'_>,
    logging: &mut Logging,
) -> Result<Result<EmbeddingCallResponse, EmbeddingCallError>, AdapterError> {
    let secrets = secret_lookup();
    let region_worker = ManagedRegionWorker::new().map_err(|error| AdapterError::Setup {
        message: error.to_string(),
    })?;
    let encoding = Counting;
    let signing = AwsSigningExecutor::default();
    let source = Source;
    let logger = |details: &Decisions| -> Result<(), PrePostError> {
        logging.capture.push(details);
        Ok(())
    };
    let prepost = env(&logging.state, &logging.redactor, &logging.sink, &logger);
    let supplied_view = execution
        .supplied
        .as_ref()
        .map(|(kind, client)| SuppliedClient {
            kind: *kind,
            transport: client,
        });
    let mut log = LogContext {
        state: &mut logging.state,
        env: &prepost,
    };
    let dependencies = Dependencies {
        secrets: &secrets,
        region_worker: &region_worker,
        source: &source,
        factory: execution.factory,
        transport: execution.transport,
        encoding: &encoding,
        signing: &signing,
        clock: Arc::new(FixedClock(clock(case.now)?)),
    };
    Ok(runtime()?.block_on(embeddings(call, dependencies, supplied_view, &mut log)))
}

fn embedding_call<'call>(
    case: &'call Case,
    api_key: Option<&'call SecretString>,
    bearer: Option<&'call SecretString>,
    extra: Option<&'call [(String, String)]>,
) -> Result<EmbeddingCall<'call>, AdapterError> {
    Ok(EmbeddingCall {
        model: &case.model,
        input: &case.input,
        optional_params: parameters(&case.optional_params)?,
        api_base: case.api_base.as_deref(),
        extra_headers: extra,
        api_key,
        timeout: case.timeout.map(Timeout::Seconds),
        aembedding: case.aembedding,
        drop_params: case.drop_params,
        environment: EmbeddingEnvironment {
            variables: &case.environment,
            runtime_endpoint: case
                .environment
                .get("AWS_BEDROCK_RUNTIME_ENDPOINT")
                .map(String::as_str),
            bearer_token: bearer,
        },
    })
}

fn bearer(case: &Case) -> Option<SecretString> {
    case.environment
        .get("AWS_BEARER_TOKEN_BEDROCK")
        .map(String::as_str)
        .map(SecretString::from)
}

fn header_pairs(headers: &BTreeMap<String, String>) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn parameters(values: &Object) -> Result<ParamMap, AdapterError> {
    let mut parameters = BTreeMap::new();
    for (name, value) in values {
        parameters.insert(
            name.clone(),
            RawValue::from_string(serde_json::to_string(value)?)?,
        );
    }
    Ok(parameters)
}

fn supplied(case: &Case, factory: &Factory) -> Option<(ClientKind, FixtureClient)> {
    let kind = match case.supplied {
        Supplied::None => return None,
        Supplied::Sync => ClientKind::Sync,
        Supplied::Async => ClientKind::Asynchronous,
    };
    Some((kind, supplied_client(factory)))
}

// the oracle case names one instant, as the pinned LiteLLM clock patch does
struct FixedClock(SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

fn clock(now: u64) -> Result<SystemTime, AdapterError> {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(now))
        .ok_or(AdapterError::Clock)
}

fn runtime() -> Result<Runtime, AdapterError> {
    Ok(Builder::new_current_thread().enable_all().build()?)
}
