#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;

use super::support::{
    COMPLETE_URL, FakeAsyncClient, FakeConfig, FakeConverter, FakeFactories, FakeSyncClient,
    Fixture, Journal, OK, Owner, PROVIDER, Recorder, Supply, VIDEO_ID, object, observed, recorder,
};
use crate::api::video_records::VideoObject;
use crate::api::video_status_transport::error::VideoStatusError;
use crate::api::video_status_transport::handler::VideoStatusHandler;
use crate::api::video_status_transport::records::{
    Headers, Params, StatusCall, SuppliedClient, VideoStatusHandled,
};

const INIT: &str = r#"{"model":"video","call_type":"video_retrieve","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

// What the handler is injected with, kept apart from the logging state its call borrows mutably
pub(crate) struct Deps {
    pub(crate) events: Recorder,
    pub(crate) config: FakeConfig,
    pub(crate) factories: FakeFactories,
    pub(crate) converter: FakeConverter,
}

impl Deps {
    pub(crate) const fn handler<'deps>(
        &'deps self,
        env: &'deps PrePostEnv<'deps>,
    ) -> VideoStatusHandler<'deps, FakeFactories> {
        VideoStatusHandler {
            factories: &self.factories,
            logging: env,
            converter: &self.converter,
        }
    }
}

pub(crate) struct Harness {
    pub(crate) deps: Deps,
    pub(crate) fixture: Fixture,
    pub(crate) state: LoggingState,
    pub(crate) supplied_sync: FakeSyncClient,
    pub(crate) supplied_async: FakeAsyncClient,
}

fn logging_state() -> LoggingState {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    let init: LoggingInit = serde_json::from_str(INIT).expect("the logging init is valid");
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default,
    )
    .expect("the logging state builds")
}

fn deps(events: &Recorder) -> Deps {
    Deps {
        events: Arc::clone(events),
        config: FakeConfig {
            events: Arc::clone(events),
            environment: Ok(Headers::new()),
            complete_url: Ok(COMPLETE_URL.to_owned()),
            transformed: Ok((COMPLETE_URL.to_owned(), Params::new())),
            response: Ok(object(VIDEO_ID)),
            asynchronous_response: Ok(object(VIDEO_ID)),
        },
        factories: FakeFactories {
            events: Arc::clone(events),
            sync: OK,
            asynchronous: OK,
            refuse_sync: false,
            refuse_async: false,
        },
        converter: FakeConverter {
            events: Arc::clone(events),
        },
    }
}

pub(crate) fn harness() -> Harness {
    let events = recorder();
    Harness {
        deps: deps(&events),
        fixture: Fixture::new(&events),
        state: logging_state(),
        supplied_sync: FakeSyncClient {
            events: Arc::clone(&events),
            owner: Owner::Supplied,
            reply: OK,
        },
        supplied_async: FakeAsyncClient {
            events: Arc::clone(&events),
            owner: Owner::Supplied,
            reply: OK,
        },
    }
}

// One status call's inputs, with the defaults every case starts from
pub(crate) struct Call<'call> {
    pub(crate) video_id: &'call str,
    pub(crate) custom_llm_provider: &'call str,
    pub(crate) litellm_params: &'call Params,
    pub(crate) extra_headers: Option<&'call Headers>,
    pub(crate) extra_body: Option<&'call Params>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) api_key: Option<&'call SecretString>,
    pub(crate) supply: Supply,
}

impl<'call> Call<'call> {
    pub(crate) const fn new(litellm_params: &'call Params) -> Self {
        Self {
            video_id: VIDEO_ID,
            custom_llm_provider: PROVIDER,
            litellm_params,
            extra_headers: None,
            extra_body: None,
            timeout: None,
            api_key: None,
            supply: Supply::None,
        }
    }

    const fn status<S, A>(
        self,
        state: &'call mut LoggingState,
        sync: &'call S,
        asynchronous: &'call A,
    ) -> StatusCall<'call, S, A> {
        let client = match self.supply {
            Supply::None => None,
            Supply::Sync => Some(SuppliedClient::Sync(sync)),
            Supply::Async => Some(SuppliedClient::Async(asynchronous)),
        };
        StatusCall {
            video_id: self.video_id,
            custom_llm_provider: self.custom_llm_provider,
            litellm_params: self.litellm_params,
            logging: state,
            extra_headers: self.extra_headers,
            extra_body: self.extra_body,
            timeout: self.timeout,
            api_key: self.api_key,
            client,
        }
    }
}

pub(crate) type Outcome = Result<VideoObject, VideoStatusError>;

impl Harness {
    pub(crate) fn events(&self) -> Journal {
        observed(&self.deps.events)
    }

    pub(crate) fn sync(&mut self, call: Call<'_>) -> Outcome {
        let env = self.fixture.environment();
        let handler = self.deps.handler(&env);
        let request = call.status(&mut self.state, &self.supplied_sync, &self.supplied_async);
        match handler.video_status_handler(&self.deps.config, request, false)? {
            VideoStatusHandled::Ready(video) => Ok(video),
            // the blocking entry never answers with a future, so one here is a broken fixture
            VideoStatusHandled::Pending(_) => Err(VideoStatusError::Suspended),
        }
    }

    pub(crate) async fn asynchronous(&mut self, call: Call<'_>) -> Outcome {
        let env = self.fixture.environment();
        let handler = self.deps.handler(&env);
        let config = &self.deps.config;
        let request = call.status(&mut self.state, &self.supplied_sync, &self.supplied_async);
        handler.async_video_status_handler(config, request).await
    }

    // The flagged entry point, with what it recorded before the future is awaited
    pub(crate) async fn flagged(&mut self, call: Call<'_>) -> (Journal, Outcome) {
        let env = self.fixture.environment();
        let handler = self.deps.handler(&env);
        let request = call.status(&mut self.state, &self.supplied_sync, &self.supplied_async);
        let pending = match handler.video_status_handler(&self.deps.config, request, true) {
            Ok(VideoStatusHandled::Pending(pending)) => pending,
            Ok(VideoStatusHandled::Ready(_)) => {
                panic!("the _is_async flag answered with a value instead of a future")
            }
            Err(error) => return (observed(&self.deps.events), Err(error)),
        };
        let before = observed(&self.deps.events);
        (before, pending.await)
    }
}
