use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_config::sections::azure_url_policy::AzureCredentials;
use betterlite_foundry::chat::sources::NoSources;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;

use super::super::call::AzureMediaOperation;
use super::super::execute::dispatch;
use super::super::runtime::MediaRuntime;
use super::speech::{FixtureError, SpeechLogging, speech_logging};
use super::{Cache, Clock, Fixture, Outcome, Recorder, Reply};
use crate::operation::OperationRequest;

pub async fn exchange_request_with<F>(
    request: OperationRequest,
    operation: AzureMediaOperation,
    replies: Vec<Reply>,
    configure: F,
) -> Result<Outcome, FixtureError>
where
    F: FnOnce(&mut SpeechLogging),
{
    let fixture = Fixture::new(replies);
    let clock = Clock;
    let sources = NoSources;
    let endpoints = EndpointSettings::default();
    let credentials = AzureCredentials::default();
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let mut cache = Cache::default();
    let mut logger = Recorder::default();
    let mut speech = speech_logging()?;
    configure(&mut speech);
    let result = {
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &speech.redactor,
            log: &speech.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: speech.now,
        };
        let mut runtime = MediaRuntime {
            endpoints: &endpoints,
            credentials: &credentials,
            environment: &environment,
            sources: &sources,
            refresh_enabled: false,
            polling_timeout: Duration::from_secs(600),
            constructor: &fixture,
            clock: &clock,
            transport: &fixture,
            cache: &mut cache,
            logger: &mut logger,
            speech_state: &mut speech.state,
            speech_env: &env,
        };
        dispatch(request, operation, &mut runtime).await
    };
    Ok(Outcome {
        result,
        fixture,
        logger,
        speech,
    })
}
