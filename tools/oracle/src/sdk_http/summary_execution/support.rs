use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::auth_model_patterns::view::ModelAccessView;
use betterlite_auth::model_budget_limiter::enforcement::BudgetSubject;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::project::ProjectTable;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::BudgetExceededDetails;
use betterlite_tokenizer::count::error::CountError;
use rust_decimal::Decimal;

use betterlite_core::api::summary_history::count::{CountedInput, HistoryTokenCounter};
use betterlite_core::api::summary_history::error::SummaryHistoryError;
use betterlite_core::api::summary_model_access::checks::{ObjectCall, SummaryModelChecks};
use betterlite_core::api::summary_model_access::error::PolicyFailure;
use betterlite_core::api::summary_model_access::repositories::SummaryAccessRepositories;
use betterlite_core::api::summary_model_limits::budget::SummaryModelBudgets;
use betterlite_core::api::summary_model_limits::error::LimiterFailure;
use betterlite_core::api::summary_model_limits::fields::BudgetMap;
use betterlite_core::api::summary_model_limits::rate::{
    BaseDescriptorRequest, DescriptorScope, LimiterCapability, OVER_LIMIT,
    PARALLEL_REQUEST_LIMITER, ProxyHooks, ReadOnlyCheck, SummaryDescriptor, SummaryRateLimiter,
};

use super::{Counter, Verdict};

// the policy step order the checks, budgets and hooks share
#[derive(Clone, Default)]
pub(super) struct Recorder(Arc<Mutex<Vec<&'static str>>>);

impl Recorder {
    fn record(&self, step: &'static str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(step);
    }

    pub(super) fn steps(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|step| (*step).to_owned())
            .collect()
    }
}

pub(super) struct ScriptedChecks {
    pub(super) recorder: Recorder,
    pub(super) verdict: Verdict,
}

impl ScriptedChecks {
    fn answer(&self) -> Result<(), PolicyFailure<io::Error>> {
        self.recorder.record("access");
        if self.verdict == Verdict::Deny {
            Err(PolicyFailure::Denied)
        } else {
            Ok(())
        }
    }
}

// the user and project checks differ only in the row they are handed, so one macro writes both
macro_rules! scripted_check {
    ($method:ident, $row_type:ty $(,)?) => {
        fn $method(
            &self,
            _router: Option<&dyn ModelAccessView>,
            _model: &str,
            _: &$row_type,
        ) -> Result<(), PolicyFailure<io::Error>> {
            self.answer()
        }
    };
}

impl SummaryModelChecks for ScriptedChecks {
    type Failure = io::Error;

    fn object_call(
        &self,
        _router: Option<&dyn ModelAccessView>,
        _call: &ObjectCall<'_>,
    ) -> Result<(), PolicyFailure<io::Error>> {
        self.answer()
    }

    scripted_check!(user_call, LiteLLMUserTable);
    scripted_check!(project_call, ProjectTable);
}

pub(super) struct NoRows;

impl SummaryAccessRepositories for NoRows {
    type Failure = io::Error;

    fn user(
        &self,
        _user_id: &str,
        _user_id_upsert: bool,
    ) -> impl Future<Output = Result<Option<LiteLLMUserTable>, io::Error>> + Send {
        ready(Ok(None))
    }

    fn project(
        &self,
        _project_id: &str,
    ) -> impl Future<Output = Result<Option<ProjectTable>, io::Error>> + Send {
        ready(Ok(None))
    }

    fn membership(
        &self,
        _user_id: &str,
        _team_id: &str,
    ) -> impl Future<Output = Result<Option<LiteLLMTeamMembership>, io::Error>> + Send {
        ready(Ok(None))
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

pub(super) struct ScriptedBudgets {
    pub(super) recorder: Recorder,
    pub(super) verdict: Verdict,
}

impl SummaryModelBudgets for ScriptedBudgets {
    fn within_model_budget(
        &self,
        _subject: &BudgetSubject<'_>,
        _model_max_budget: &BudgetMap,
        _model: &str,
    ) -> impl Future<Output = Result<(), ErrorClass>> + Send {
        self.recorder.record("budget");
        ready(if self.verdict == Verdict::Deny {
            Err(exceeded())
        } else {
            Ok(())
        })
    }
}

pub(super) struct ScriptedLimiter {
    pub(super) verdict: Verdict,
}

impl ScriptedLimiter {
    fn descriptor() -> SummaryDescriptor {
        SummaryDescriptor {
            key: String::from("key"),
            value: String::from("value"),
            rate_limit: None,
        }
    }
}

impl SummaryRateLimiter for ScriptedLimiter {
    type Failure = io::Error;

    fn supports(&self, _capability: LimiterCapability) -> bool {
        true
    }

    fn create_rate_limit_descriptors(
        &self,
        _request: &BaseDescriptorRequest<'_>,
    ) -> Result<Vec<SummaryDescriptor>, LimiterFailure<io::Error>> {
        Ok(vec![Self::descriptor()])
    }

    fn add_scope_descriptors(
        &self,
        _scope: DescriptorScope,
        _principal: &UserAPIKeyAuth,
        _requested_model: &str,
        _descriptors: &mut Vec<SummaryDescriptor>,
    ) -> Result<(), LimiterFailure<io::Error>> {
        Ok(())
    }

    fn should_rate_limit(
        &self,
        _check: &ReadOnlyCheck<'_>,
    ) -> impl Future<Output = Result<Option<String>, LimiterFailure<io::Error>>> + Send {
        let code = if self.verdict == Verdict::Deny {
            OVER_LIMIT.to_owned()
        } else {
            String::from("OK")
        };
        ready(Ok(Some(code)))
    }
}

pub(super) struct ScriptedHooks {
    pub(super) recorder: Recorder,
    pub(super) limiter: ScriptedLimiter,
}

impl ProxyHooks for ScriptedHooks {
    type Limiter = ScriptedLimiter;
    type Error = io::Error;

    fn get_proxy_hook(&self, hook: &str) -> Result<Option<&ScriptedLimiter>, io::Error> {
        self.recorder.record("rate");
        if hook == PARALLEL_REQUEST_LIMITER {
            Ok(Some(&self.limiter))
        } else {
            Ok(None)
        }
    }
}

pub(super) struct ScriptedCounter {
    pub(super) script: Counter,
}

impl HistoryTokenCounter for ScriptedCounter {
    fn count(
        &self,
        _model: &str,
        input: CountedInput<'_>,
    ) -> impl Future<Output = Result<usize, SummaryHistoryError>> + Send {
        let tokens = match input {
            CountedInput::Conversation { .. } => self.script.conversation,
            CountedInput::Text(_text) => self.script.text,
        };
        let outcome = if self.script.fail {
            Err(SummaryHistoryError::TokenCount(CountError::MissingInput))
        } else {
            Ok(tokens)
        };
        ready(outcome)
    }
}
