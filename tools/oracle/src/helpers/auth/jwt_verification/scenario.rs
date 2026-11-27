use std::sync::{Arc, Mutex};

use betterlite_auth::jwt_keys::cache::KeyCache;
use betterlite_auth::jwt_keys::clock::Clock;
use betterlite_auth::jwt_verification::errors::{VerificationError, into_proxy_exception};
use betterlite_auth::jwt_verification::keys::PublicKey;
use betterlite_auth::jwt_verification::settings::JwtVerificationSettings;
use betterlite_auth::jwt_verification::userinfo::UserinfoHttp;
use betterlite_auth::jwt_verification::verifier::JwtVerifier;
use betterlite_config::sections::jwt_settings::Settings as JwtAuthSettings;
use betterlite_config::sections::jwt_settings::groups::{
    ClaimFields, KeyFields, RbacFields, RouteFields, RoutingFields, ScopeFields, SyncFields,
    TeamFields, UserinfoFields, VirtualKeyFields,
};
use betterlite_config::sections::jwt_settings::records::Selector;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use tokio::runtime::Builder;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::fakes::{Cache, FixedClock, ScriptedUserinfo, Source, lock, tail};
use super::input::{Issuer, Op, Scenario, Step};
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
    let audience = scenario
        .audience
        .as_deref()
        .map(|text| Selector::One(text.to_owned()));
    let cache = Arc::new(Cache::default());
    let http = Arc::new(ScriptedUserinfo::default());
    let clock: Arc<dyn Clock> = Arc::new(FixedClock { now: scenario.now });
    let cache_tier: Arc<dyn KeyCache> = Arc::<Cache>::clone(&cache);
    let userinfo: Arc<dyn UserinfoHttp> = Arc::<ScriptedUserinfo>::clone(&http);
    let verifier = JwtVerifier::new(
        Arc::new(settings_for(scenario)),
        cache_tier,
        userinfo,
        clock,
    );
    let source = Source {
        key: scenario.key.clone(),
    };
    let fixture = Fixture {
        verifier: &verifier,
        source: &source,
        cache: &cache,
        http: &http,
        audience: audience.as_ref(),
        issuer: scenario.issuer.as_deref(),
        levels,
    };
    let mut steps = Vec::with_capacity(scenario.steps.len());
    for step in &scenario.steps {
        steps.push(step_answer(&fixture, step).await);
    }
    ScenarioAnswer { steps }
}

fn settings_for(scenario: &Scenario) -> JwtVerificationSettings {
    let issuers: Vec<_> = scenario.issuers.iter().map(Issuer::config).collect();
    let jwt = JwtAuthSettings {
        routes: RouteFields::default(),
        teams: TeamFields::default(),
        claims: ClaimFields::default(),
        keys: KeyFields::default(),
        rbac: RbacFields::default(),
        scopes: ScopeFields::default(),
        custom_validate: None,
        sync: SyncFields::default(),
        userinfo: UserinfoFields {
            oidc_userinfo_endpoint: scenario.userinfo_endpoint.clone(),
            oidc_userinfo_enabled: false,
            oidc_userinfo_cache_ttl: scenario.userinfo_ttl.unwrap_or(300.0),
        },
        virtual_keys: VirtualKeyFields::default(),
        routing: RoutingFields {
            routing_overrides: None,
            team_claim_fallback: false,
            fallback_to_db_teams: false,
            issuers: Some(issuers),
        },
    };
    JwtVerificationSettings {
        audience: scenario.audience.clone(),
        issuer: scenario.issuer.clone(),
        jwt,
    }
}

// the fakes one scenario drives, so a step answers from one borrow instead of six
struct Fixture<'scenario> {
    verifier: &'scenario JwtVerifier,
    source: &'scenario Source,
    cache: &'scenario Cache,
    http: &'scenario ScriptedUserinfo,
    audience: Option<&'scenario Selector>,
    issuer: Option<&'scenario str>,
    levels: &'scenario Mutex<Vec<String>>,
}

async fn step_answer(fixture: &Fixture<'_>, step: &Step) -> StepAnswer {
    let warnings_from = lock(fixture.levels).len();
    let requests_from = lock(&fixture.http.requests).len();
    let writes_from = lock(&fixture.cache.writes).len();
    let token = step.token.as_deref().unwrap_or_default();
    let (value, error) = match step.op {
        Op::AuthJwt => auth_answer(fixture, token).await,
        Op::Decode => decode_answer(fixture, token),
        Op::Userinfo => userinfo_answer(fixture, token).await,
    };
    StepAnswer {
        value,
        error,
        warnings: tail(fixture.levels, warnings_from),
        requests: tail(&fixture.http.requests, requests_from),
        writes: tail(&fixture.cache.writes, writes_from),
    }
}

async fn auth_answer(fixture: &Fixture<'_>, token: &str) -> (Json, Option<Failure>) {
    match fixture.verifier.auth_jwt(token, fixture.source).await {
        Ok(payload) => (json_value(payload.json), None),
        Err(error) => (Json::Null, Some(auth_failure(&error))),
    }
}

fn decode_answer(fixture: &Fixture<'_>, token: &str) -> (Json, Option<Failure>) {
    let Some(key) = PublicKey::from_resolved(&fixture.source.key) else {
        return (
            Json::Null,
            Some(auth_failure(&VerificationError::InvalidJwt)),
        );
    };
    match fixture.verifier.decode_with_public_key(
        token,
        &key,
        fixture.audience,
        fixture.issuer,
        None,
        false,
    ) {
        Ok(payload) => (json_value(payload.json), None),
        Err(error) => (Json::Null, Some(auth_failure(&error))),
    }
}

// TypedValue carries serde_json's value, and the answer reports this crate's JSON type.
fn json_value(value: serde_json::Value) -> Json {
    match value {
        serde_json::Value::Null => Json::Null,
        serde_json::Value::Bool(flag) => Json::Bool(flag),
        serde_json::Value::Number(number) => Json::Number(number),
        serde_json::Value::String(text) => Json::Text(text),
        serde_json::Value::Array(items) => Json::List(items.into_iter().map(json_value).collect()),
        serde_json::Value::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(field, value)| (field, json_value(value)))
                .collect(),
        ),
    }
}

async fn userinfo_answer(fixture: &Fixture<'_>, token: &str) -> (Json, Option<Failure>) {
    match fixture.verifier.get_oidc_userinfo(token).await {
        Ok(fields) => (Json::Object(fields), None),
        Err(_) => (Json::Null, Some(Failure::default())),
    }
}

fn auth_failure(error: &VerificationError) -> Failure {
    let exception = into_proxy_exception(error);
    Failure {
        status: exception.code.parse().ok(),
        error_type: Some(exception.error_type),
        param: exception.param,
        code: Some(exception.code),
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
