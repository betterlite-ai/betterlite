use super::bedrock::converse_call::{
    ConverseAssembly, ConverseCall, ConverseDeps, ConverseOutput, converse_call,
};
use super::bedrock::converse_request::ConverseMedia;
use super::bedrock::invoke_events::DecoderClock;
use super::input::{Input, Observation};
use super::metadata::{Offline, metadata};
use super::setup::{Quiet, call, remote, state};
use super::transport::{Signer, Source, Transport};
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::StreamExt as _;
use secrecy::SecretString;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub(crate) enum FixtureError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Map(#[from] betterlite_models::model_metadata_resolution::names::ModelCostMapError),
    #[error(transparent)]
    Patterns(#[from] betterlite_context::anthropic_catalog_lookup::error::CatalogLookupError),
    #[error(transparent)]
    Log(#[from] betterlite_logging::logging_state::LoggingStateError),
    #[error(transparent)]
    Redact(#[from] betterlite_redact::error::RedactionError),
    #[error(transparent)]
    Http(#[from] betterlite_http::error::HttpError),
    #[error(transparent)]
    Runtime(#[from] io::Error),
}

pub(crate) fn run(input: &Input) -> Result<Observation, FixtureError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(execute(input))
}

struct Session {
    transport: Transport,
    source: Source,
    redactor: Redactor,
}

fn session(input: &Input) -> Result<Session, FixtureError> {
    Ok(Session {
        transport: Transport {
            input: input.clone(),
            seen: Arc::new(Mutex::new(Observation::default())),
            reads: Arc::new(AtomicUsize::new(0)),
        },
        source: Source(AtomicUsize::new(0)),
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?,
    })
}

async fn execute(input: &Input) -> Result<Observation, FixtureError> {
    let session = session(input)?;
    let mut logging = state(&session.redactor)?;
    let logs = Mutex::new(Vec::new());
    let logger = |details: &betterlite_models::decisions::Json| {
        logs.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(details.clone());
        Ok(())
    };
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &session.redactor,
        log: &Quiet,
        input_callbacks: &[],
        sentry: None,
        logger_fn: Some(&logger),
        now: logging.start_time,
    };
    let log = super::bedrock::image_call::plan::LogContext {
        state: &mut logging,
        env: &env,
    };
    let mut observed = call_body(input, &session, log).await?;
    observed.logs = logs.into_inner().unwrap_or_else(PoisonError::into_inner);
    Ok(observed)
}

async fn call_body(
    input: &Input,
    session: &Session,
    log: super::bedrock::image_call::plan::LogContext<'_, '_>,
) -> Result<Observation, FixtureError> {
    let bearer = input.bearer.clone().map(SecretString::from);
    let headers: Vec<(String, String)> = input
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let mut request = call(input, bearer.as_ref(), &headers);
    let messages: Vec<betterlite_prompts::message_content::Message> =
        serde_json::from_str(r#"[{"role":"user","content":"hello"}]"#)?;
    request.messages = &messages;
    let mut observed = Observation::default();
    match send_request(&mut request, session, log).await? {
        Ok(output) => collect(output, input.consume, &mut observed).await,
        Err(error) => {
            observed.status = error.status();
            observed.error = Some(format!("{error:?}: {error}"));
        }
    }
    let mut seen = session
        .transport
        .seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    observed.url = std::mem::take(&mut seen.url);
    observed.body = std::mem::take(&mut seen.body);
    observed.signed_body = std::mem::take(&mut seen.signed_body);
    observed.headers = std::mem::take(&mut seen.headers);
    observed.drops = seen.drops;
    observed.sends = seen.sends;
    observed.factory = seen.factory;
    observed.factory_timeout_ms = seen.factory_timeout_ms;
    observed.credentials = session.source.0.load(Ordering::Relaxed);
    observed.reads = session.transport.reads.load(Ordering::Relaxed);
    Ok(observed)
}

async fn send_request<'run>(
    request: &mut ConverseCall<'_>,
    session: &'run Session,
    log: super::bedrock::image_call::plan::LogContext<'run, 'run>,
) -> Result<
    Result<ConverseOutput<'run>, super::bedrock::converse_call::error::ConverseCallError>,
    FixtureError,
> {
    let betas = BetaMatrix(Default::default());
    let supplied_buffered = session.transport.input.native.supplied.map(|kind| {
        super::bedrock::rerank::transport::SuppliedClient {
            kind,
            transport: &session.transport,
        }
    });
    let supplied_stream = session.transport.input.native.supplied.map(|kind| {
        super::bedrock::converse_call::transport::SuppliedStream {
            kind,
            client: super::bedrock::converse_call::transport::StreamClient {
                handler: Arc::new(session.transport.clone()),
                timeout: Duration::from_secs(7),
            },
        }
    });
    let deps = ConverseDeps {
        credentials: &session.source,
        signer: &Signer(&session.transport),
        factory: &session.transport,
        supplied_buffered,
        supplied_stream,
        ids: Arc::new(Quiet),
        clock: DecoderClock {
            next_id: Uuid::nil,
            now: || 0,
        },
        redactor: &session.redactor,
        betas: &betas,
        now: SystemTime::UNIX_EPOCH,
        reasoning_counter: &Quiet,
        state: log.state,
        env: log.env,
    };
    assembled(request, deps).await
}

async fn assembled<'stream>(
    request: &mut ConverseCall<'_>,
    deps: ConverseDeps<'_, 'stream, Transport, Source>,
) -> Result<
    Result<ConverseOutput<'stream>, super::bedrock::converse_call::error::ConverseCallError>,
    FixtureError,
> {
    let fixture = metadata()?;
    let resolution = fixture.resolution();
    let mut keys = ModelCostKeys::default();
    let mut capabilities = fixture.capabilities(&resolution, &mut keys);
    let media = remote()?;
    let catalogs = fixture.catalogs();
    let model_cost = Default::default();
    let assembly = ConverseAssembly {
        environment: &mut capabilities,
        lookup: &Offline,
        media: ConverseMedia {
            remote: &media,
            new_id: &|| "fixture-tool".to_owned(),
        },
        model_cost: &model_cost,
        catalogs: &catalogs,
        packaged: &fixture.packaged,
    };
    Ok(converse_call(request, deps, assembly).await)
}

async fn collect(output: ConverseOutput<'_>, consume: bool, observed: &mut Observation) {
    match output {
        ConverseOutput::Reply(reply) => {
            observed.reasoning_tokens = reply
                .response
                .usage
                .as_ref()
                .and_then(|usage| usage.completion_tokens_details.reasoning_tokens);
            observed.response_headers = reply.provider_headers.headers.into_iter().collect();
            observed.content = reply
                .response
                .choices
                .first()
                .and_then(|choice| choice.message.content.clone())
        }
        ConverseOutput::Stream(mut stream) => {
            observed.stream = true;
            observed.wrapper_model = Some(stream.model.clone());
            observed.provider = Some(stream.custom_llm_provider.to_owned());
            observed.response_headers = stream.response_headers.clone().into_iter().collect();
            observed.chunk_size = stream.stream_chunk_size;
            if consume {
                while let Some(chunk) = stream.completion_stream.next().await {
                    match chunk {
                        Ok(_) => observed.chunks = observed.chunks.saturating_add(1),
                        Err(error) => {
                            observed.status = error.status();
                            observed.error = Some(format!("{error:?}: {error}"));
                        }
                    }
                }
            }
        }
    }
}
