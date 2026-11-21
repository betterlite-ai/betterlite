mod budgets;
mod limiter;

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_core::api::summary_model_limits::budget::summary_model_within_budget;
use betterlite_core::api::summary_model_limits::error::SummaryLimitsError;
use betterlite_core::api::summary_model_limits::rate::{
    SummaryDescriptor, summary_model_within_rate_limit,
};
use betterlite_db::budget_permission_rows::JsonData;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::runtime::Builder;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use budgets::spend_limiter;
use limiter::{Call, RateScript, ScriptedHooks, ScriptedLimiter};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    summary_model: String,
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    principal: Option<BTreeMap<String, JsonData>>,
    spend: BTreeMap<String, serde_json::Number>,
    offline: Vec<String>,
    rate: RateScript,
}

#[derive(Serialize)]
struct Outcome {
    budget: bool,
    rate: Option<bool>,
    raised: Option<String>,
    reads: Vec<String>,
    calls: Vec<Call>,
    retained: Option<Vec<SummaryDescriptor>>,
    warnings: Vec<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let mut outcomes = Vec::with_capacity(input.scenarios.len());
        for scenario in &input.scenarios {
            let warnings = Warnings::default();
            let guard = set_default(registry().with(warnings.clone()));
            let mut outcome = runtime.block_on(evaluate(&input.summary_model, scenario))?;
            drop(guard);
            outcome.warnings = warnings.lines();
            outcomes.push(outcome);
        }
        Ok(json!({ "outcomes": outcomes }))
    }
}

async fn evaluate(summary_model: &str, scenario: &Scenario) -> Result<Outcome, Box<HelperError>> {
    let principal = scenario
        .principal
        .clone()
        .map(|fields| UserAPIKeyAuth::construct(AuthInput::Dict(fields)))
        .transpose()
        .or_value_error()?;
    let budgets = spend_limiter(&scenario.spend, &scenario.offline)?;
    let journal = Mutex::new(Vec::new());
    let hooks = ScriptedHooks {
        script: &scenario.rate,
        journal: &journal,
        limiter: ScriptedLimiter {
            script: &scenario.rate,
            journal: &journal,
            retained: Mutex::new(None),
        },
    };
    let budget = summary_model_within_budget(principal.as_ref(), Some(&budgets), summary_model)
        .await
        .or_value_error()?;
    let checked =
        summary_model_within_rate_limit(principal.as_ref(), Some(&hooks), summary_model).await;
    let (rate, raised) = match checked {
        Ok(allowed) => (Some(allowed), None),
        Err(SummaryLimitsError::Hook(failure)) => (None, Some(failure.to_string())),
        Err(error @ SummaryLimitsError::Principal(_)) => return Err(error).or_value_error(),
    };
    let retained = hooks
        .limiter
        .retained
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    Ok(Outcome {
        budget,
        rate,
        raised,
        reads: budgets.limiter.cache.reads(),
        calls: journal.into_inner().unwrap_or_else(PoisonError::into_inner),
        retained,
        warnings: Vec::new(),
    })
}

#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

impl Warnings {
    fn lines(&self) -> Vec<String> {
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
