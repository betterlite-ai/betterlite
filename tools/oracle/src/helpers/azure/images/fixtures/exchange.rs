use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_azure::image_transport::SystemClock;
use betterlite_azure::images::error::ImageError;
use betterlite_azure::images::generation::{
    GeneratedImage, Generation, Runtime, begin, finish, send,
};
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_config::sections::azure_image_transport::Settings as TransportSettings;
use secrecy::SecretString;
use tokio::runtime::Builder;
use tokio::time::Instant;

use super::sources::NoSources;
use super::transport::{FixtureState, FixtureTransport, RecordedCall, RecordingLogger};
use crate::images::input::Step;

type GenerationRun = (
    Result<GeneratedImage, ImageError>,
    Vec<RecordedCall>,
    RecordingLogger,
);

fn generation<'data>(
    step: &'data Step,
    api_key: Option<&'data SecretString>,
    token: Option<&'data SecretString>,
) -> Generation<'data> {
    Generation {
        model: step.model.as_deref(),
        prompt: &step.prompt,
        timeout: Some(600.0),
        optional_params: step.optional.clone(),
        headers: step.headers.clone().into_iter().collect(),
        api_key,
        api_base: step.api_base.as_deref(),
        api_version: step.api_version.as_deref(),
        azure_ad_token: token,
        azure_ad_token_provider: None,
        base_model: step.base_model.as_deref(),
        params: LiteLlmParams::default(),
        delegate_async: false,
        now_seconds: 1_700_000_000,
    }
}

pub(crate) fn run_generation(step: &Step) -> Option<GenerationRun> {
    let state = Arc::new(FixtureState {
        calls: Mutex::default(),
        reply: Mutex::new(step.reply_body.clone()),
    });
    let transport = FixtureTransport {
        state: Arc::clone(&state),
    };
    let mut logger = RecordingLogger::default();
    let endpoints = EndpointSettings::default();
    let polling_timeout = TransportSettings::default().operation_polling_timeout;
    let api_key = step.api_key.as_deref().map(SecretString::from);
    let token = step.azure_ad_token.as_deref().map(SecretString::from);
    let call = generation(step, api_key.as_ref(), token.as_ref());
    let clock = SystemClock {
        origin: Instant::now(),
    };
    let environment = BTreeMap::<String, String>::new();
    let sources = NoSources;
    let mut runtime = Runtime {
        endpoints: &endpoints,
        environment: &environment,
        sources: &sources,
        refresh_enabled: false,
        polling_timeout,
        constructor: &transport,
        clock: &clock,
        logger: &mut logger,
    };
    let result = exchange(call, &mut runtime)?;
    let calls = state
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Some((result, calls, logger))
}

// the three production steps around the one await, driven by a current-thread runtime
fn exchange(
    call: Generation<'_>,
    runtime: &mut Runtime<'_, FixtureTransport, SystemClock>,
) -> Option<Result<GeneratedImage, ImageError>> {
    let driver = Builder::new_current_thread().enable_all().build().ok()?;
    Some(match begin(call, runtime) {
        Ok(begun) => {
            let sent = driver.block_on(send(
                &begun,
                runtime.constructor,
                runtime.clock,
                runtime.polling_timeout,
            ));
            finish(&begun, sent, runtime.logger)
        }
        Err(error) => Err(error),
    })
}
