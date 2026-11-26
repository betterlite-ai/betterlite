use std::sync::{Arc, Mutex};

use betterlite_auth::jwt_keys::cache::KeyCache;
use betterlite_auth::jwt_keys::clock::Clock;
use betterlite_auth::jwt_keys::errors::{KeyError, exception_for};
use betterlite_auth::jwt_keys::http::JwksHttp;
use betterlite_auth::jwt_keys::resolver::JwksResolver;
use betterlite_auth::jwt_keys::settings::JwtKeySettings;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Value, json};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::fakes::{Cache, Scripted, StepClock, lock};
use super::input::{Scenario, Step};

pub(super) fn answer(scenarios: &[Scenario]) -> HelperResult {
    let levels = Arc::new(Mutex::new(Vec::new()));
    let subscriber = registry().with(RecordLevels {
        levels: Arc::clone(&levels),
    });
    let _guard = tracing::subscriber::set_default(subscriber);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    let mut results = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        results.push(runtime.block_on(scenario_answer(scenario, &levels))?);
    }
    Ok(json!({ "scenarios": results }))
}

async fn scenario_answer(
    scenario: &Scenario,
    levels: &Mutex<Vec<String>>,
) -> Result<Value, Box<HelperError>> {
    let Ok(settings) = JwtKeySettings::new(
        scenario.settings.public_key_url.clone(),
        scenario.settings.public_key_ttl,
        scenario.settings.public_key_stale_ttl,
        None,
        None,
    ) else {
        return Ok(json!({"settings_error": true, "steps": []}));
    };
    let clock = Arc::new(StepClock::default());
    let cache = Arc::new(Cache::new(Arc::clone(&clock)));
    let http = Arc::new(Scripted::default());
    let cache_tier: Arc<dyn KeyCache> = Arc::clone(&cache);
    let http_source: Arc<dyn JwksHttp> = Arc::clone(&http);
    let wall_clock: Arc<dyn Clock> = Arc::clone(&clock);
    let resolver = JwksResolver::new(Arc::new(settings), cache_tier, http_source, wall_clock);
    let mut steps = Vec::with_capacity(scenario.steps.len());
    for step in &scenario.steps {
        steps.push(step_answer(&resolver, &clock, &cache, &http, levels, step).await);
    }
    Ok(json!({"settings_error": false, "steps": steps}))
}

async fn step_answer(
    resolver: &JwksResolver,
    clock: &StepClock,
    cache: &Cache,
    http: &Scripted,
    levels: &Mutex<Vec<String>>,
    step: &Step,
) -> Value {
    *lock(&clock.now) = step.now;
    lock(&http.replies).extend(step.http.iter().cloned());
    let requested = lock(&http.requests).len();
    let slept = lock(&clock.sleeps).len();
    let written = lock(&cache.writes).len();
    let noticed = lock(levels).len();
    let kid = step.kid.as_deref();
    let outcome = if step.concurrent {
        let (first, second) =
            tokio::join!(resolver.get_public_key(kid), resolver.get_public_key(kid));
        first.and(second)
    } else {
        resolver.get_public_key(kid).await
    };
    let (value, error) = match outcome {
        Ok(served) => (served, Json::Null),
        Err(error) => (Json::Null, error_view(&error)),
    };
    json!({
        "value": value,
        "error": error,
        "requests": lock(&http.requests).iter().skip(requested).cloned().collect::<Vec<String>>(),
        "sleeps": lock(&clock.sleeps).iter().skip(slept).copied().collect::<Vec<f64>>(),
        "writes": lock(&cache.writes).iter().skip(written).cloned().collect::<Vec<Value>>(),
        "warnings": lock(levels).iter().skip(noticed).cloned().collect::<Vec<String>>(),
    })
}

// The stage answers a bare key failure as 401 and an outage as the retryable 503, as both proxies do.
fn error_view(error: &KeyError) -> Value {
    let exception = exception_for(error);
    let status = exception.code.parse::<u16>().unwrap_or(0_u16);
    json!({
        "status": status,
        "type": exception.error_type,
        "param": exception.param,
        "code": exception.code,
    })
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
