use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::batch_limit_descriptors::Settings;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::api::api_boundaries::ApiLimits;
use crate::api::batch_limit_descriptors::BatchLimitDescriptors;
use crate::api::batch_limit_descriptors::descriptors::{
    BatchDescriptorRateLimit, BatchLimitDescriptor,
};
use crate::api::batch_limit_descriptors::input::BatchLimitDescriptorsInput;
use crate::api::batch_limit_descriptors::limiter::BatchLimiterSource;
use crate::api::model_credentials::CredentialSource;
use crate::api::request_destination::RequestData;

pub(super) const ITPM: &str = "model_per_project_itpm";
pub(super) const OTPM: &str = "model_per_project_otpm";

// A typed input assembled from two JSON fragments, as the case files spell them
pub(super) fn input(principal_fields: &str, calls: &str) -> BatchLimitDescriptorsInput {
    let wire = format!(r#"{{"principal": {principal_fields}, "calls": {calls}}}"#);
    serde_json::from_str(&wire).expect("the input fixture decodes")
}

pub(super) fn settings(
    disable: Option<bool>,
    providers: Option<Vec<String>>,
    models: Option<Vec<String>>,
) -> Settings {
    Settings {
        disable_rate_limiting: disable,
        skip_providers: providers,
        skip_models: models,
    }
}

pub(super) fn descriptor(
    key: &str,
    value: &str,
    requests: Option<i64>,
    tokens: Option<i64>,
) -> BatchLimitDescriptor {
    BatchLimitDescriptor {
        key: key.to_owned(),
        value: value.to_owned(),
        rate_limit: Some(BatchDescriptorRateLimit {
            requests_per_unit: requests,
            tokens_per_unit: tokens,
            max_parallel_requests: None,
            window_size: Some(60),
        }),
    }
}

pub(super) fn credentials(provider: &str) -> JsonObject {
    let mut fields = JsonObject::new();
    fields.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    );
    fields
}

#[derive(Debug, thiserror::Error)]
#[error("the limiter is offline")]
pub(super) struct LimiterFailure;

#[derive(Debug, thiserror::Error)]
#[error("the credential source is offline")]
pub(super) struct SourceFailure;

pub(super) struct FakeLimiter {
    pub descriptors: Vec<BatchLimitDescriptor>,
    pub project: Vec<BatchLimitDescriptor>,
    pub candidates: i64,
    pub floor: i64,
    pub failure: bool,
}

pub(super) fn limiter(
    descriptors: Vec<BatchLimitDescriptor>,
    project: Vec<BatchLimitDescriptor>,
    candidates: i64,
    floor: i64,
    failure: bool,
) -> FakeLimiter {
    FakeLimiter {
        descriptors,
        project,
        candidates,
        floor,
        failure,
    }
}

impl BatchLimiterSource for FakeLimiter {
    type Error = LimiterFailure;

    const PROJECT_OTPM_DESCRIPTOR_KEY: &'static str = OTPM;

    fn descriptors_for(
        &self,
        _auth: &UserAPIKeyAuth,
        _data: &RequestData,
    ) -> Result<Vec<BatchLimitDescriptor>, LimiterFailure> {
        if self.failure {
            return Err(LimiterFailure);
        }
        Ok(self.descriptors.clone())
    }

    fn project_io_for(
        &self,
        _auth: &UserAPIKeyAuth,
        model: &str,
        out: &mut Vec<BatchLimitDescriptor>,
    ) {
        for descriptor in &self.project {
            let mut record = descriptor.clone();
            record.value = format!("{model}:{}", descriptor.value);
            out.push(record);
        }
    }

    fn candidate_count(&self, _body: &RequestData) -> i64 {
        self.candidates
    }

    fn output_floor(&self, _min_configured_otpm_limit: Option<i64>) -> i64 {
        self.floor
    }
}

pub(super) struct FakeCredentials {
    pub models: BTreeMap<String, Option<JsonObject>>,
    pub default: Option<JsonObject>,
    pub failure: bool,
    pub asked: Mutex<Vec<String>>,
}

pub(super) fn router(
    models: BTreeMap<String, Option<JsonObject>>,
    default: Option<JsonObject>,
    failure: bool,
) -> FakeCredentials {
    FakeCredentials {
        models,
        default,
        failure,
        asked: Mutex::new(Vec::new()),
    }
}

impl FakeCredentials {
    pub(super) fn asked(&self) -> Vec<String> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CredentialSource for FakeCredentials {
    type Error = SourceFailure;

    fn credentials_with_provider(
        &self,
        model_id: &str,
        _team_id: Option<&str>,
    ) -> Result<Option<JsonObject>, SourceFailure> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model_id.to_owned());
        if self.failure {
            return Err(SourceFailure);
        }
        Ok(self
            .models
            .get(model_id)
            .cloned()
            .unwrap_or_else(|| self.default.clone()))
    }
}

pub(super) fn service(
    settings: Settings,
    limiter: FakeLimiter,
    credentials: Option<FakeCredentials>,
) -> BatchLimitDescriptors<FakeLimiter, FakeCredentials> {
    BatchLimitDescriptors {
        settings,
        limiter,
        credentials,
        limits: ApiLimits::default(),
        warned_unsupported_model_skip: AtomicBool::new(false),
    }
}

#[derive(Clone)]
pub(super) struct LevelLog {
    level: Level,
    lines: Arc<Mutex<Vec<String>>>,
}

impl LevelLog {
    pub(super) fn warnings() -> Self {
        Self::new(Level::WARN)
    }

    pub(super) fn debugs() -> Self {
        Self::new(Level::DEBUG)
    }

    fn new(level: Level) -> Self {
        Self {
            level,
            lines: Arc::default(),
        }
    }

    pub(super) fn lines(&self) -> Vec<String> {
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for LevelLog {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() == self.level {
            let mut message = Message::default();
            event.record(&mut message);
            self.lines
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(message.0);
        }
    }
}
