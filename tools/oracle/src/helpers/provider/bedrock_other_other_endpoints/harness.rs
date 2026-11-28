use std::error::Error as StdError;
use std::sync::Arc;
use std::time::SystemTime;

use betterlite_bedrock::count_call::handler::CountTokensHandler;
use betterlite_bedrock::embedding_call::Clock;
use betterlite_bedrock::signing::executor::AwsSigningExecutor;
use betterlite_bedrock::transcription::binding::TranscriptionBinding;
use betterlite_provider::bedrock_image_dispatch::test_support::Catalog;
use betterlite_provider::bedrock_other_dispatch::call::BedrockOtherOperation;
use betterlite_provider::bedrock_other_dispatch::error::BedrockOtherError;
use betterlite_provider::bedrock_other_dispatch::execute::{LogContext, Services, dispatch};
use betterlite_provider::bedrock_other_dispatch::test_support::client::{FixtureClient, Replies};
use betterlite_provider::bedrock_other_dispatch::test_support::factory::Factory;
use betterlite_provider::bedrock_other_dispatch::test_support::record::{Recorded, Seen};
use betterlite_provider::bedrock_other_dispatch::test_support::source::Source;
use betterlite_provider::bedrock_other_dispatch::test_support::{
    Counting, DiscardingSink, Files, FixedClock, NOW_SECONDS, instant, logging_state, prepost_env,
    redactor, region_worker, secret_lookup,
};
use betterlite_provider::bedrock_other_dispatch::{BedrockOtherRequest, BedrockOtherResult};
use betterlite_provider::operation::{OperationRequest, OperationResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use super::model::{Failed, Probe, credentials, replies};
use crate::openai_media_dispatch::{OTHER_OUTCOME, failure};

// The scripted transport, factory and identity one probe runs the production dispatcher against
pub(super) struct Harness {
    pub(super) factory: Factory,
    pub(super) transport: FixtureClient,
    pub(super) source: Source,
    replies: Replies,
    seen: Seen,
}

impl Harness {
    pub(super) fn new(probe: &Probe) -> Result<Self, Failed> {
        let answers = replies(probe);
        let replies = Replies::queue(&answers);
        let seen = Seen::default();
        let mut factory = Factory::queued(&answers);
        factory.replies = replies.clone();
        factory.seen = seen.clone();
        let transport = FixtureClient::new(replies.clone(), seen.clone());
        Ok(Self {
            factory,
            transport,
            source: Source::new(credentials(probe)?),
            replies,
            seen,
        })
    }

    pub(super) fn client(&self) -> FixtureClient {
        FixtureClient::new(self.replies.clone(), self.seen.clone())
    }
}

// What one dispatch observed: its outcome, every posted request, and the regions it resolved
pub(super) struct Outcome<E> {
    pub(super) result: Result<BedrockOtherResult, BedrockOtherError<E>>,
    pub(super) requests: Vec<Recorded>,
    pub(super) regions: Vec<String>,
}

pub(super) async fn exchange<H, B>(
    harness: &Harness,
    operation: BedrockOtherOperation<'_, '_, '_, H, B>,
) -> Result<Outcome<B::Error>, Failed>
where
    H: CountTokensHandler,
    B: TranscriptionBinding + Sync,
{
    let redactor =
        Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| failure("Redactor", error))?;
    let mut state = logging_state(&redactor).map_err(|error| failure("Logging", error))?;
    let sink = DiscardingSink;
    let env = prepost_env(&state, &redactor, &sink);
    let secrets = secret_lookup();
    let workers = region_worker().map_err(|error| failure("RegionWorker", error))?;
    let signing = AwsSigningExecutor::default();
    let encoding = Counting;
    let files = Files;
    let clock: Arc<dyn Clock> = Arc::new(FixedClock(
        instant(NOW_SECONDS).map_err(|error| failure("Instant", error))?,
    ));
    let services = Services {
        secrets: &secrets,
        region_worker: &workers,
        source: &harness.source,
        factory: &harness.factory,
        transport: &harness.transport,
        encoding: &encoding,
        signing: &signing,
        clock,
        files: &files,
    };
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let dispatched = dispatch(
        OperationRequest::BedrockOther(BedrockOtherRequest),
        operation,
        &services,
        &mut log,
    )
    .await;
    Ok(Outcome {
        result: outcome(dispatched)?,
        requests: harness.factory.requests(),
        regions: harness.source.regions(),
    })
}

fn outcome<E>(
    dispatched: Result<OperationResult, BedrockOtherError<E>>,
) -> Result<Result<BedrockOtherResult, BedrockOtherError<E>>, Failed>
where
    E: StdError + 'static,
{
    match dispatched {
        Ok(OperationResult::BedrockOther(result)) => Ok(Ok(result)),
        Ok(_other) => Err(failure("Variant", OTHER_OUTCOME)),
        Err(error) => Ok(Err(error)),
    }
}

pub(super) fn now() -> Result<SystemTime, Failed> {
    instant(NOW_SECONDS).map_err(|error| failure("Instant", error))
}

pub(super) fn empty_catalog() -> Result<Catalog, Failed> {
    Catalog::empty().map_err(|error| failure("Catalog", error))
}
