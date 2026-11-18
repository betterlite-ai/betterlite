use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::auth_read::budget_failure::{
    BudgetLookupFailure, FailureClass, log_budget_lookup_failure,
};
use betterlite_auth::auth_read::membership::{
    SharedMembershipLoad, member_allowed_models, membership_from_shared_load,
};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde::Deserialize;
use serde_json::json;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    MemberModelScope {
        membership: LiteLLMTeamMembership,
    },
    SharedTyped {
        membership: LiteLLMTeamMembership,
    },
    SharedSubtype {
        membership: LiteLLMTeamMembership,
    },
    SharedForeign {
        payload: CacheValue,
    },
    BudgetFailureLogPolicy {
        exact_exception: bool,
        message: String,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::MemberModelScope { membership } => {
                Ok(json!({"allowed_models": member_allowed_models(membership)}))
            }
            Input::SharedTyped { membership } | Input::SharedSubtype { membership } => {
                Ok(shared_report(SharedMembershipLoad::Membership(Box::new(
                    membership.clone(),
                ))))
            }
            Input::SharedForeign { payload } => Ok(shared_report(SharedMembershipLoad::Foreign(
                Box::new(payload.clone()),
            ))),
            Input::BudgetFailureLogPolicy {
                exact_exception,
                message,
            } => failure_report(*exact_exception, message.as_str()),
        }
    }
}

fn shared_report(load: SharedMembershipLoad) -> serde_json::Value {
    match membership_from_shared_load(load) {
        Some(membership) => json!({"accepted": true, "user_id": membership.user_id}),
        None => json!({"accepted": false, "user_id": null}),
    }
}

fn failure_report(exact_exception: bool, message: &str) -> HelperResult {
    let captured = Records::default();
    let subscriber = tracing_subscriber::registry().with(captured.clone());
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?;
    let class = if exact_exception {
        FailureClass::Exception
    } else {
        FailureClass::Named
    };
    let failure = BudgetLookupFailure { class, message };
    with_default(subscriber, || {
        log_budget_lookup_failure(&redactor, "user", &failure);
    });
    let records = captured
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let first = records.first();
    Ok(json!({
        "records": records.len(),
        "level": first.map(|record| record.level.as_str()),
        "hint": records.iter().any(|record| !record.hint.is_empty()),
    }))
}

#[derive(Clone, Default)]
struct Records(Arc<Mutex<Vec<Record>>>);

#[derive(Clone)]
struct Record {
    level: tracing::Level,
    hint: String,
}

impl<Inner: Subscriber> Layer<Inner> for Records {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut hint = HintField::default();
        event.record(&mut hint);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Record {
                level: *event.metadata().level(),
                hint: hint.0,
            });
    }
}

#[derive(Default)]
struct HintField(String);

impl Visit for HintField {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "hint" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "hint" {
            self.0 = format!("{value:?}");
        }
    }
}
