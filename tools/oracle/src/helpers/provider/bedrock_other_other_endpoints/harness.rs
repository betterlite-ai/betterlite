use std::collections::VecDeque;
use std::error::Error as StdError;
use std::sync::{Arc, Mutex};

use betterlite_bedrock::count_call::handler::{BedrockCountTokensHandler, CountTokensHandler};
use betterlite_bedrock::transcription::binding::TranscriptionBinding;
use betterlite_models::decisions::Json;
use betterlite_provider::bedrock_other_dispatch::BedrockOtherRequest;
use betterlite_provider::bedrock_other_dispatch::BedrockOtherResult;
use betterlite_provider::bedrock_other_dispatch::call::BedrockOtherOperation;
use betterlite_provider::bedrock_other_dispatch::error::BedrockOtherError;
use betterlite_provider::bedrock_other_dispatch::execute::{LogContext, Services, dispatch};
use betterlite_provider::bedrock_other_dispatch::test_support::client::{
    FixtureClient, Replies, Reply,
};
use betterlite_provider::bedrock_other_dispatch::test_support::factory::{ClientCall, Factory};
use betterlite_provider::bedrock_other_dispatch::test_support::record::{Recorded, Seen};
use betterlite_provider::bedrock_other_dispatch::test_support::source::Source;
use betterlite_provider::bedrock_other_dispatch::test_support::{
    Capture, DiscardingSink, Prepared, capturing_env,
};
use betterlite_provider::operation::{OperationRequest, OperationResult};

use super::model::{Failed, Probe, credentials, replies};
use crate::openai_media_dispatch::{OTHER_OUTCOME, failure};

pub(super) type Handler = BedrockCountTokensHandler<FixtureClient, Source>;

// The scripted transport, factory and identity one probe runs the production dispatcher against
pub(super) struct Harness {
    pub(super) factory: Factory,
    pub(super) transport: FixtureClient,
    pub(super) source: Source,
    pub(super) replies: Replies,
    pub(super) seen: Seen,
}

impl Harness {
    pub(super) fn new(probe: &Probe) -> Result<Self, Failed> {
        let answers = replies(probe);
        let queue = Replies(Arc::new(Mutex::new(
            answers.iter().cloned().collect::<VecDeque<Reply>>(),
        )));
        let seen = Seen::default();
        let mut factory = Factory::queued(&answers);
        factory.replies = queue.clone();
        factory.seen = seen.clone();
        let transport = FixtureClient {
            replies: queue.clone(),
            seen: seen.clone(),
        };
        Ok(Self {
            factory,
            transport,
            source: Source::new(credentials(probe)?),
            replies: queue,
            seen,
        })
    }
}

// What one dispatch observed: its outcome, every posted request, the clients it asked for and the phases it logged
pub(super) struct Outcome<E>
where
    E: StdError + 'static,
{
    pub(super) result: Result<BedrockOtherResult, BedrockOtherError<E>>,
    pub(super) requests: Vec<Recorded>,
    pub(super) calls: Vec<ClientCall>,
    pub(super) regions: Vec<String>,
    pub(super) phases: Vec<String>,
}

pub(super) async fn exchange<H, B>(
    harness: &Harness,
    operation: BedrockOtherOperation<'_, '_, '_, H, B>,
) -> Result<Outcome<B::Error>, Failed>
where
    H: CountTokensHandler,
    B: TranscriptionBinding + Send + Sync,
{
    let mut prepared = Prepared::new().map_err(|error| failure("Prepared", error))?;
    let sink = DiscardingSink;
    let capture = Capture::default();
    let logger = |details: &Json| capture.record(details);
    let env = capturing_env(&prepared.state, &prepared.redactor, &sink, &logger);
    let services = Services {
        secrets: &prepared.secrets,
        region_worker: &prepared.workers,
        source: &harness.source,
        factory: &harness.factory,
        transport: &harness.transport,
        encoding: &prepared.encoding,
        signing: &prepared.signing,
        clock: Arc::clone(&prepared.clock),
        files: &prepared.files,
    };
    let mut log = LogContext {
        state: &mut prepared.state,
        env: &env,
    };
    let dispatched = dispatch(
        OperationRequest::BedrockOther(BedrockOtherRequest),
        operation,
        &services,
        &mut log,
    )
    .await;
    observe(harness, &capture, dispatched)
}

// What the dispatch observed, including the clients the family asked for and the phases it logged
fn observe<E>(
    harness: &Harness,
    capture: &Capture,
    dispatched: Result<OperationResult, BedrockOtherError<E>>,
) -> Result<Outcome<E>, Failed>
where
    E: StdError + 'static,
{
    let result = match dispatched {
        Ok(OperationResult::BedrockOther(result)) => Ok(Ok(result)),
        Ok(_other) => Err(failure("Variant", OTHER_OUTCOME)),
        Err(error) => Ok(Err(error)),
    }?;
    Ok(Outcome {
        result,
        requests: harness.factory.seen.requests(),
        calls: harness.factory.calls(),
        regions: harness.source.regions(),
        phases: capture.phases(),
    })
}
