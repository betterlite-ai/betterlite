use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::model_budget_limiter::enforcement::BudgetSubject;
use betterlite_auth::resolved_key::auth::{AuthInput, ParentOtelSpan, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::error::details::BudgetExceededDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use rust_decimal::Decimal;
use serde::Deserialize;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::api::summary_model_limits::budget::SummaryModelBudgets;
use crate::api::summary_model_limits::error::LimiterFailure;
use crate::api::summary_model_limits::fields::BudgetMap;
use crate::api::summary_model_limits::rate::{
    BaseDescriptorRequest, LimiterCapability, ProxyHooks, ReadOnlyCheck, SummaryDescriptor,
    SummaryRateLimiter,
};

pub(super) fn principal(fields: serde_json::Value) -> UserAPIKeyAuth {
    let map: BTreeMap<String, JsonData> = serde_json::from_value(fields).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).unwrap()
}

pub(super) fn budget(model: &str) -> BudgetMap {
    let limit = serde_json::json!({"budget_limit": 1.0, "time_period": "1d"});
    BTreeMap::from([(model.to_owned(), serde_json::from_value(limit).unwrap())])
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BudgetCall {
    pub scope: &'static str,
    pub id: String,
    pub alias: Option<String>,
    pub key_budget: Option<BudgetMap>,
    pub budget: BudgetMap,
    pub model: String,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Verdict {
    Exceeded,
    Offline,
}

#[derive(Default)]
pub(super) struct FakeBudgets {
    pub failing: Option<(&'static str, Verdict)>,
    pub calls: Mutex<Vec<BudgetCall>>,
}

impl FakeBudgets {
    pub(super) fn calls(&self) -> Vec<BudgetCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl SummaryModelBudgets for FakeBudgets {
    fn within_model_budget(
        &self,
        subject: &BudgetSubject<'_>,
        model_max_budget: &BudgetMap,
        model: &str,
    ) -> impl Future<Output = Result<(), ErrorClass>> + Send {
        let (scope, id, alias, key_budget) = match subject {
            BudgetSubject::Key {
                entity_id,
                key_alias,
            } => ("key", *entity_id, key_alias.map(str::to_owned), None),
            BudgetSubject::User { user_id } => ("user", *user_id, None, None),
            BudgetSubject::Team {
                team_id,
                key_model_max_budget,
            } => ("team", *team_id, None, key_model_max_budget.cloned()),
            BudgetSubject::EndUser { end_user_id } => ("end-user", *end_user_id, None, None),
        };
        self.calls.lock().unwrap().push(BudgetCall {
            scope,
            id: id.to_owned(),
            alias,
            key_budget,
            budget: model_max_budget.clone(),
            model: model.to_owned(),
        });
        let outcome = match self.failing {
            Some((failing, Verdict::Exceeded)) if failing == scope => Err(exceeded()),
            Some((failing, Verdict::Offline)) if failing == scope => Err(offline()),
            Some(_) | None => Ok(()),
        };
        ready(outcome)
    }
}

fn exceeded() -> ErrorClass {
    ErrorClass::BudgetExceeded(Box::new(BudgetExceededDetails {
        current_cost: Decimal::TWO,
        max_budget: Decimal::ONE,
        message: Some("exceeded".to_owned()),
        provider: String::new(),
        entity_type: None,
        entity_id: None,
        status_code: 429,
    }))
}

fn offline() -> ErrorClass {
    ErrorClass::Unclassified(Box::new(ErrorContext {
        message: "offline".to_owned(),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }))
}

pub(super) fn descriptor(key: &str, limits: Option<serde_json::Value>) -> SummaryDescriptor {
    SummaryDescriptor {
        key: key.to_owned(),
        value: format!("{key}-value"),
        rate_limit: limits.map(|raw| serde_json::from_value(raw).unwrap()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Stage {
    Create,
    Team,
    Project,
    Organization,
    Check,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum LimiterCall {
    Create {
        token: Option<String>,
        model: String,
        rpm_limit_type: Option<JsonData>,
        tpm_limit_type: Option<JsonData>,
        model_has_failures: bool,
    },
    Team {
        token: Option<String>,
        model: String,
        seen: Vec<SummaryDescriptor>,
    },
    Project {
        token: Option<String>,
        model: String,
        seen: Vec<SummaryDescriptor>,
    },
    Organization {
        token: Option<String>,
        model: String,
    },
    Check {
        descriptors: Vec<SummaryDescriptor>,
        parent_otel_span: Option<JsonData>,
        read_only: bool,
    },
}

#[derive(Clone, Copy, Debug)]
pub(super) enum RateFailure {
    Unverifiable,
    Offline,
}

#[derive(Debug, thiserror::Error)]
#[error("offline")]
pub(super) struct Offline;

#[derive(Default)]
pub(super) struct FakeLimiter {
    pub missing: Vec<LimiterCapability>,
    pub base: Vec<SummaryDescriptor>,
    pub team: Vec<SummaryDescriptor>,
    pub project: Vec<SummaryDescriptor>,
    pub organization: Vec<SummaryDescriptor>,
    pub overall_code: Option<String>,
    pub failure: Option<(Stage, RateFailure)>,
    pub calls: Mutex<Vec<LimiterCall>>,
}

impl FakeLimiter {
    pub(super) fn calls(&self) -> Vec<LimiterCall> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, stage: Stage, call: LimiterCall) -> Result<(), LimiterFailure<Offline>> {
        self.calls.lock().unwrap().push(call);
        match self.failure {
            Some((failing, RateFailure::Unverifiable)) if failing == stage => {
                Err(LimiterFailure::Unverifiable {
                    detail: "unverified".to_owned(),
                })
            }
            Some((failing, RateFailure::Offline)) if failing == stage => {
                Err(LimiterFailure::Other(Offline))
            }
            Some(_) | None => Ok(()),
        }
    }
}

#[derive(Deserialize)]
struct TokenField {
    token: Option<String>,
}

fn token(principal: &UserAPIKeyAuth) -> Option<String> {
    let fields: TokenField =
        serde_json::from_value(serde_json::to_value(principal).unwrap()).unwrap();
    fields.token
}

impl SummaryRateLimiter for FakeLimiter {
    type Failure = Offline;

    fn supports(&self, capability: LimiterCapability) -> bool {
        !self.missing.contains(&capability)
    }

    fn create_rate_limit_descriptors(
        &self,
        request: &BaseDescriptorRequest<'_>,
    ) -> Result<Vec<SummaryDescriptor>, LimiterFailure<Offline>> {
        let call = LimiterCall::Create {
            token: token(request.principal),
            model: request.model.to_owned(),
            rpm_limit_type: request.rpm_limit_type.cloned(),
            tpm_limit_type: request.tpm_limit_type.cloned(),
            model_has_failures: request.model_has_failures,
        };
        self.record(Stage::Create, call)?;
        Ok(self.base.clone())
    }

    fn add_team_model_descriptor(
        &self,
        principal: &UserAPIKeyAuth,
        requested_model: &str,
        descriptors: &mut Vec<SummaryDescriptor>,
    ) -> Result<(), LimiterFailure<Offline>> {
        let call = LimiterCall::Team {
            token: token(principal),
            model: requested_model.to_owned(),
            seen: descriptors.clone(),
        };
        self.record(Stage::Team, call)?;
        descriptors.extend(self.team.iter().cloned());
        Ok(())
    }

    fn add_project_model_descriptor(
        &self,
        principal: &UserAPIKeyAuth,
        requested_model: &str,
        descriptors: &mut Vec<SummaryDescriptor>,
    ) -> Result<(), LimiterFailure<Offline>> {
        let call = LimiterCall::Project {
            token: token(principal),
            model: requested_model.to_owned(),
            seen: descriptors.clone(),
        };
        self.record(Stage::Project, call)?;
        descriptors.extend(self.project.iter().cloned());
        Ok(())
    }

    fn create_organization_descriptors(
        &self,
        principal: &UserAPIKeyAuth,
        requested_model: &str,
    ) -> Result<Vec<SummaryDescriptor>, LimiterFailure<Offline>> {
        let call = LimiterCall::Organization {
            token: token(principal),
            model: requested_model.to_owned(),
        };
        self.record(Stage::Organization, call)?;
        Ok(self.organization.clone())
    }

    fn should_rate_limit(
        &self,
        check: &ReadOnlyCheck<'_>,
    ) -> impl Future<Output = Result<Option<String>, LimiterFailure<Offline>>> + Send {
        let span = check.parent_otel_span.map(|span| match span {
            ParentOtelSpan::Json(data) => data.clone(),
            ParentOtelSpan::Opaque => JsonData::String("opaque".to_owned()),
        });
        let call = LimiterCall::Check {
            descriptors: check.descriptors.to_vec(),
            parent_otel_span: span,
            read_only: check.read_only,
        };
        let outcome = self
            .record(Stage::Check, call)
            .map(|()| self.overall_code.clone());
        ready(outcome)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("hooks offline")]
pub(super) struct HooksOffline;

pub(super) struct FakeHooks {
    pub limiter: Option<FakeLimiter>,
    pub raises: bool,
    pub requested: Mutex<Vec<String>>,
}

impl ProxyHooks for FakeHooks {
    type Limiter = FakeLimiter;
    type Error = HooksOffline;

    fn get_proxy_hook(&self, hook: &str) -> Result<Option<&FakeLimiter>, HooksOffline> {
        self.requested.lock().unwrap().push(hook.to_owned());
        if self.raises {
            return Err(HooksOffline);
        }
        Ok(self.limiter.as_ref())
    }
}

#[derive(Clone, Default)]
pub(super) struct Warnings(Arc<Mutex<Vec<String>>>);

impl Warnings {
    pub(super) fn lines(&self) -> Vec<String> {
        self.0
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

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() == Level::WARN {
            let mut message = Message::default();
            event.record(&mut message);
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(message.0);
        }
    }
}
