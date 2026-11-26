use std::mem;
use std::sync::{Arc, Mutex};

use betterlite_auth::jwt_keys::cache::KeyCache;
use betterlite_auth::jwt_keys::clock::Clock;
use betterlite_auth::jwt_keys::errors::{KeyError, exception_for};
use betterlite_auth::jwt_keys::http::JwksHttp;
use betterlite_auth::jwt_keys::resolver::JwksResolver;
use betterlite_auth::jwt_keys::settings::JwtKeySettings;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use tokio::runtime::Builder;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::fakes::{Cache, Scripted, StepClock, lock};
use super::input::{Scenario, Step};
use super::output::{Answer, Failure, ScenarioAnswer, StepAnswer};

pub(super) fn answer(scenarios: &[Scenario]) -> HelperResult {
    let levels = Arc::new(Mutex::new(Vec::new()));
    let subscriber = registry().with(RecordLevels {
        levels: Arc::clone(&levels),
    });
    let _guard = set_default(subscriber);
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    let mut results = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        results.push(runtime.block_on(scenario_answer(scenario, &levels)));
    }
    serde_json::to_value(Answer { scenarios: results }).or_value_error()
}

async fn scenario_answer(scenario: &Scenario, levels: &Mutex<Vec<String>>) -> ScenarioAnswer {
    let Ok(settings) = JwtKeySettings::new(
        scenario.settings.public_key_url.clone(),
        scenario.settings.public_key_ttl,
        scenario.settings.public_key_stale_ttl,
        None,
        None,
    ) else {
        return ScenarioAnswer {
            settings_error: true,
            steps: Vec::new(),
        };
    };
    let clock = Arc::new(StepClock::default());
    let cache = Arc::new(Cache::new(Arc::clone(&clock)));
    let http = Arc::new(Scripted::default());
    let cache_tier: Arc<dyn KeyCache> = Arc::<Cache>::clone(&cache);
    let http_source: Arc<dyn JwksHttp> = Arc::<Scripted>::clone(&http);
    let wall_clock: Arc<dyn Clock> = Arc::<StepClock>::clone(&clock);
    let resolver = JwksResolver::new(Arc::new(settings), cache_tier, http_source, wall_clock);
    let fixture = Fixture {
        clock: clock.as_ref(),
        cache: cache.as_ref(),
        http: http.as_ref(),
        levels,
    };
    let mut steps = Vec::with_capacity(scenario.steps.len());
    for step in &scenario.steps {
        steps.push(step_answer(&resolver, &fixture, step).await);
    }
    ScenarioAnswer {
        settings_error: false,
        steps,
    }
}

// the fakes a step drives, so a step answers from one borrow instead of five
struct Fixture<'a> {
    clock: &'a StepClock,
    cache: &'a Cache,
    http: &'a Scripted,
    levels: &'a Mutex<Vec<String>>,
}

async fn step_answer(resolver: &JwksResolver, fixture: &Fixture<'_>, step: &Step) -> StepAnswer {
    *lock(&fixture.clock.now) = step.now;
    lock(&fixture.http.replies).extend(step.http.iter().cloned());
    let kid = step.kid.as_deref();
    let outcome = if step.concurrent {
        let (first, second) =
            tokio::join!(resolver.get_public_key(kid), resolver.get_public_key(kid));
        first.and(second)
    } else {
        resolver.get_public_key(kid).await
    };
    let (value, error) = match outcome {
        Ok(served) => (served, None),
        Err(failure) => (Json::Null, Some(error_view(&failure))),
    };
    StepAnswer {
        value,
        error,
        requests: mem::take(&mut *lock(&fixture.http.requests)),
        sleeps: mem::take(&mut *lock(&fixture.clock.sleeps)),
        writes: mem::take(&mut *lock(&fixture.cache.writes)),
        warnings: mem::take(&mut *lock(fixture.levels)),
    }
}

// The stage answers a bare key failure as 401 and an outage as the retryable 503, as both proxies do.
fn error_view(error: &KeyError) -> Failure {
    let exception = exception_for(error);
    Failure {
        status: exception.code.parse().unwrap_or(0_u16),
        error_type: exception.error_type,
        param: exception.param,
        code: exception.code,
    }
}

struct RecordLevels {
    levels: Arc<Mutex<Vec<String>>>,
}

// Python's logger drops debug and info below its level, so only the WARN events the proxy logger names are kept.
impl<S: tracing::Subscriber> Layer<S> for RecordLevels {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        let metadata = event.metadata();
        if metadata.target() == "LiteLLM Proxy" && *metadata.level() == tracing::Level::WARN {
            lock(&self.levels).push("warning".to_owned());
        }
    }
}
