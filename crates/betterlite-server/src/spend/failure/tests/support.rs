use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_auth::reservation_settlement::adjust::AdjustmentMode;
use betterlite_auth::reservation_settlement::error::SettlementError;
use betterlite_auth::reservation_settlement::handle::ReservationHandle;
use betterlite_auth::reservation_settlement::reconcile::ReservationFinality;
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_error::normalize::Normalizer;
use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_logging::spend_bodies::session::SpendTimestamp;
use betterlite_logging::spend_sanitize::SpendSanitizePolicy;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::super::enrich::{FailureIdentityLookup, IdentityLookupError, KeyFacts};
use super::super::record::FailureSettings;
use super::super::write::{FailureSpend, FailureSpendWrite};
use crate::spend::estimates::ports::Clock;
use crate::spend::settlement::ports::ReservationSettlement;

pub(super) fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("the fixture is an object")
}

pub(super) fn nested<'json>(root: &'json JsonObject, path: &[&str]) -> &'json Json {
    let (head, rest) = path.split_first().expect("the fixture path is not empty");
    let next = root.get(*head).expect("the fixture keeps the path");
    match (rest.is_empty(), next) {
        (true, _) => next,
        (false, Json::Object(fields)) => nested(fields, rest),
        (false, _) => panic!("the fixture path crosses a non-object"),
    }
}

pub(super) fn policy(store_bodies: bool, max_string_length: i64) -> SpendSanitizePolicy {
    SpendSanitizePolicy {
        store_bodies,
        max_string_length,
    }
}

pub(super) fn settings() -> FailureSettings {
    FailureSettings {
        disable_error_logs: false,
        tracebacks: betterlite_config::collector::TracebackPolicy::default(),
        sanitize: policy(false, 2048),
    }
}

pub(super) fn principal() -> PrincipalFacts {
    PrincipalFacts {
        api_key_hash: Some("hashed-key".to_owned()),
        key_alias: Some("test-key".to_owned()),
        user_id: Some("user-1".to_owned()),
        team_id: Some("team-1".to_owned()),
        org_id: Some("org-1".to_owned()),
        project_id: Some("project-1".to_owned()),
        ..PrincipalFacts::default()
    }
}

pub(super) fn normalizer() -> Normalizer {
    Normalizer::new().expect("the normalizer builds")
}

pub(super) struct Harness {
    pub request_data: JsonObject,
    pub principal: PrincipalFacts,
    pub facts: ExceptionFacts,
    pub normalizer: Normalizer,
}

impl Harness {
    pub(super) fn new() -> Self {
        Self {
            request_data: object(r#"{"model": "gpt-4o"}"#),
            principal: principal(),
            facts: ExceptionFacts {
                class_name: "ProviderError".to_owned(),
                message: Some("the provider is down".to_owned()),
                status_code: Some(503_u16),
                ..ExceptionFacts::default()
            },
            normalizer: normalizer(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Written {
    pub request_data: JsonObject,
    pub response_result: String,
    pub response_cost: Decimal,
    pub token: Option<String>,
    pub user_id: Option<String>,
    pub end_user_id: Option<String>,
    pub team_id: Option<String>,
    pub org_id: Option<String>,
    pub project_id: Option<String>,
    pub start_time: SpendTimestamp,
    pub end_time: SpendTimestamp,
}

#[derive(Default)]
pub(super) struct RecordingWriter {
    pub charged: bool,
    written: Mutex<Vec<Written>>,
}

impl RecordingWriter {
    pub(super) const fn new(charged: bool) -> Self {
        Self {
            charged,
            written: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn written(&self) -> Vec<Written> {
        self.written.lock().expect("the writer lock").clone()
    }
}

#[async_trait]
impl FailureSpendWrite for RecordingWriter {
    async fn record(&self, spend: &FailureSpend<'_>) -> bool {
        let written = Written {
            request_data: spend.request_data.clone(),
            response_result: spend.response_result.to_owned(),
            response_cost: spend.response_cost,
            token: spend.token.map(str::to_owned),
            user_id: spend.user_id.map(str::to_owned),
            end_user_id: spend.end_user_id.map(str::to_owned),
            team_id: spend.team_id.map(str::to_owned),
            org_id: spend.org_id.map(str::to_owned),
            project_id: spend.project_id.map(str::to_owned),
            start_time: spend.start_time,
            end_time: spend.end_time,
        };
        self.written.lock().expect("the writer lock").push(written);
        self.charged
    }
}

#[derive(Clone, Copy)]
pub(super) struct FixedClock(pub DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

pub(super) struct Settlement {
    pub release_ok: bool,
    pub invalidate_ok: bool,
    pub calls: Mutex<Vec<&'static str>>,
}

impl Settlement {
    pub(super) const fn new(release_ok: bool, invalidate_ok: bool) -> Self {
        Self {
            release_ok,
            invalidate_ok,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().expect("the settlement lock").clone()
    }

    fn record(&self, call: &'static str, ok: bool) -> Result<(), SettlementError> {
        self.calls.lock().expect("the settlement lock").push(call);
        if ok {
            Ok(())
        } else {
            Err(SettlementError::Overflow)
        }
    }
}

#[async_trait]
impl ReservationSettlement for Settlement {
    async fn reconcile(
        &self,
        _reservation: &ReservationHandle,
        _actual_cost: Decimal,
        _finality: ReservationFinality,
        _mode: AdjustmentMode,
    ) -> Result<(), SettlementError> {
        Ok(())
    }

    async fn release(&self, _reservation: &ReservationHandle) -> Result<(), SettlementError> {
        self.record("release", self.release_ok)
    }

    async fn invalidate(&self, _reservation: &ReservationHandle) -> Result<(), SettlementError> {
        self.record("invalidate", self.invalidate_ok)
    }
}

pub(super) fn reservation() -> ReservationHandle {
    ReservationHandle::new(Decimal::from(5_i32), Vec::new(), Decimal::ZERO)
}

#[derive(Default)]
pub(super) struct SpyLookup {
    pub key_calls: Mutex<u32>,
    pub team_calls: Mutex<u32>,
}

impl SpyLookup {
    pub(super) fn key_calls(&self) -> u32 {
        *self.key_calls.lock().expect("the key counter")
    }

    pub(super) fn team_calls(&self) -> u32 {
        *self.team_calls.lock().expect("the team counter")
    }
}

#[async_trait]
impl FailureIdentityLookup for SpyLookup {
    async fn key_facts(&self, _hashed_token: &str) -> Result<KeyFacts, IdentityLookupError> {
        let mut calls = self.key_calls.lock().expect("the key counter");
        *calls = calls.saturating_add(1);
        Ok(KeyFacts {
            key_alias: Some("looked-up-alias".to_owned()),
            user_id: Some("looked-up-user".to_owned()),
            ..KeyFacts::default()
        })
    }

    async fn team_alias(&self, _team_id: &str) -> Result<Option<String>, IdentityLookupError> {
        let mut calls = self.team_calls.lock().expect("the team counter");
        *calls = calls.saturating_add(1);
        Ok(Some("looked-up-team".to_owned()))
    }
}
