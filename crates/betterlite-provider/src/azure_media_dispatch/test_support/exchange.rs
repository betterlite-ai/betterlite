use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_config::sections::azure_url_policy::AzureCredentials;
use betterlite_foundry::chat::sources::NoSources;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;

use super::super::call::AzureMediaOperation;
use super::super::error::AzureMediaError;
use super::super::execute::dispatch;
use super::super::runtime::MediaRuntime;
use super::speech::{FixtureError, SpeechLogging, speech_logging};
use super::{Cache, Clock, Fixture, Outcome, Recorder, Reply};
use crate::operation::{OperationRequest, OperationResult};

// The fixture state one exchange runs against, built once and then borrowed by the dispatch.
struct Exchange {
    fixture: Fixture,
    clock: Clock,
    sources: NoSources,
    endpoints: EndpointSettings,
    credentials: AzureCredentials,
    environment: BTreeMap<String, String>,
    cache: Cache,
    logger: Recorder,
    speech: SpeechLogging,
}

impl Exchange {
    fn new(replies: Vec<Reply>) -> Result<Self, FixtureError> {
        Ok(Self {
            fixture: Fixture::new(replies),
            clock: Clock,
            sources: NoSources,
            endpoints: EndpointSettings::default(),
            credentials: AzureCredentials::default(),
            environment: BTreeMap::new(),
            cache: Cache::default(),
            logger: Recorder::default(),
            speech: speech_logging()?,
        })
    }

    async fn dispatch(
        &mut self,
        request: OperationRequest,
        operation: AzureMediaOperation,
    ) -> Result<OperationResult, AzureMediaError> {
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.speech.redactor,
            log: &self.speech.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: self.speech.now,
        };
        let mut runtime = MediaRuntime {
            endpoints: &self.endpoints,
            credentials: &self.credentials,
            environment: &self.environment,
            sources: &self.sources,
            refresh_enabled: false,
            polling_timeout: Duration::from_secs(600),
            constructor: &self.fixture,
            clock: &self.clock,
            transport: &self.fixture,
            cache: &mut self.cache,
            logger: &mut self.logger,
            speech_state: &mut self.speech.state,
            speech_env: &env,
        };
        dispatch(request, operation, &mut runtime).await
    }
}

pub async fn exchange_request_with<F>(
    request: OperationRequest,
    operation: AzureMediaOperation,
    replies: Vec<Reply>,
    configure: F,
) -> Result<Outcome, FixtureError>
where
    F: FnOnce(&mut SpeechLogging),
{
    let mut exchange = Exchange::new(replies)?;
    configure(&mut exchange.speech);
    let result = exchange.dispatch(request, operation).await;
    Ok(Outcome {
        result,
        fixture: exchange.fixture,
        logger: exchange.logger,
        speech: exchange.speech,
    })
}
