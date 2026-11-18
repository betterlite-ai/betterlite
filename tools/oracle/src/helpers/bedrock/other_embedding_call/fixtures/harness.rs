use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::bedrock_embedding_io::credentials::ManagedRegionWorker;
use betterlite_bedrock::bedrock_embedding_io::transport::{ClientKind, SuppliedClient, Timeout};
use betterlite_bedrock::embedding_call::error::EmbeddingCallError;
use betterlite_bedrock::embedding_call::plan::remaining_options;
use betterlite_bedrock::embedding_call::{
    Dependencies, EmbeddingCall, EmbeddingCallResponse, EmbeddingEnvironment, LogContext, ParamMap,
};
use betterlite_bedrock::signing::executor::AwsSigningExecutor;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;
use serde_json::value::RawValue;
use tokio::runtime::Builder;

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

pub(crate) fn run(case: &Case) -> Result<Harness, AdapterError> {
    let factory = Factory::new(&case.replies);
    let transport = supplied_client(&factory);
    let supplied = supplied(case, &factory);
    let api_key = case.api_key.as_deref().map(SecretString::from);
    let bearer = case
        .environment
        .get("AWS_BEARER_TOKEN_BEDROCK")
        .map(SecretString::from);
    let extra: Option<Vec<(String, String)>> = case.extra_headers.as_ref().map(|headers| {
        headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    });
    let mut call = EmbeddingCall {
        model: &case.model,
        input: &case.input,
        optional_params: parameters(&case.optional_params)?,
        api_base: case.api_base.as_deref(),
        extra_headers: extra.as_deref(),
        api_key: api_key.as_ref(),
        timeout: case.timeout.map(Timeout::Seconds),
        aembedding: case.aembedding,
        drop_params: case.drop_params,
        environment: EmbeddingEnvironment {
            variables: &case.environment,
            runtime_endpoint: case
                .environment
                .get("AWS_BEDROCK_RUNTIME_ENDPOINT")
                .map(String::as_str),
            bearer_token: bearer.as_ref(),
        },
        now: clock(case.now)?,
    };
    let secrets = secret_lookup();
    let region_worker = ManagedRegionWorker::new().map_err(|error| AdapterError::Setup {
        message: error.to_string(),
    })?;
    let encoding = Counting;
    let signing = AwsSigningExecutor::default();
    let source = Source;
    let redactor = redactor()?;
    let sink = DiscardingSink;
    let mut state = state(&redactor)?;
    let capture = Capture::default();
    let logger = |details: &betterlite_models::decisions::Json| capture.push(details);
    let prepost = env(&state, &redactor, &sink, &logger);
    let supplied_view = supplied.as_ref().map(|(kind, client)| SuppliedClient {
        kind: *kind,
        transport: client,
    });
    let outcome = {
        let mut log = LogContext {
            state: &mut state,
            env: &prepost,
        };
        let dependencies = Dependencies {
            secrets: &secrets,
            region_worker: &region_worker,
            source: &source,
            factory: &factory,
            transport: &transport,
            encoding: &encoding,
            signing: &signing,
        };
        runtime()?.block_on(betterlite_bedrock::embedding_call::embeddings(
            &mut call,
            dependencies,
            supplied_view,
            &mut log,
        ))
    };
    let mut requests = factory.requests();
    requests.extend(supplied.iter().flat_map(|(_, client)| client.requests()));
    requests.extend(transport.requests());
    Ok(Harness {
        outcome,
        requests,
        factory: factory.calls(),
        phases: capture.phases(),
        remaining: remaining_options(&call.optional_params)?,
    })
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
        Supplied::Async => ClientKind::Async,
    };
    Some((kind, supplied_client(factory)))
}

fn clock(now: u64) -> Result<SystemTime, AdapterError> {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(now))
        .ok_or(AdapterError::Clock)
}

fn runtime() -> Result<tokio::runtime::Runtime, AdapterError> {
    Ok(Builder::new_current_thread().enable_all().build()?)
}
