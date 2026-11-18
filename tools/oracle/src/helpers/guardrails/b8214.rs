use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::local_ttl_cache::Settings as CacheSettings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_guardrails::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use betterlite_guardrails::semantic_guard::semantic_guard_route_loading::builder::SemanticRouterBuilder;
use betterlite_guardrails::semantic_guard::semantic_guard_route_loading::route::Route;
use betterlite_guardrails::semantic_guard_hook::init::SemanticGuardArgs;
use betterlite_guardrails::semantic_guard_hook::scoring::{RouteChoice, SemanticRouteScorer};
use betterlite_guardrails::semantic_guard_hook::{GUARDRAIL_PROVIDER, SemanticGuardrail};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio::sync::Mutex;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    settings: Settings,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    guardrail_name: String,
    embedding_model: String,
    similarity_threshold: f64,
    custom_routes: Vec<GuardrailValue>,
    on_flagged_action: String,
    default_on: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    request: BTreeMap<String, GuardrailValue>,
    call_type: String,
    matches: bool,
    response: Option<GuardrailValue>,
}

// the embedding service is the only step a case cannot run, so the case scripts which route the router answers
struct ScriptedRouter {
    route_name: Option<String>,
    matches: Arc<Mutex<bool>>,
    scored: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl SemanticRouteScorer for ScriptedRouter {
    type Error = Infallible;

    async fn score(&self, text: &str) -> Result<Vec<RouteChoice>, Self::Error> {
        self.scored.lock().await.push(text.to_owned());
        let matched = *self.matches.lock().await;
        let choice = if matched {
            RouteChoice {
                name: self.route_name.clone(),
                similarity_score: Some(1.0),
            }
        } else {
            RouteChoice {
                name: None,
                similarity_score: None,
            }
        };
        Ok(vec![choice])
    }
}

struct ScriptedBuilder {
    matches: Arc<Mutex<bool>>,
    scored: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl SemanticRouterBuilder for ScriptedBuilder {
    type Router = ScriptedRouter;

    type Error = Infallible;

    async fn build(
        &self,
        routes: Vec<Route>,
        _global_threshold: f64,
    ) -> Result<Self::Router, Self::Error> {
        Ok(ScriptedRouter {
            route_name: routes.first().map(|route| route.name.clone()),
            matches: Arc::clone(&self.matches),
            scored: Arc::clone(&self.scored),
        })
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure("OSError", &error))?;
        let matches = Arc::new(Mutex::new(false));
        let scored = Arc::new(Mutex::new(Vec::new()));
        let builder = ScriptedBuilder {
            matches: Arc::clone(&matches),
            scored: Arc::clone(&scored),
        };
        let built = runtime
            .block_on(SemanticGuardrail::new(
                guardrail_args(&input.settings),
                &builder,
                true,
            ))
            .or_value_error()?;
        let tool_cache = cache()?;
        let mut reports = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            reports.push(runtime.block_on(one_call(&built, &matches, &scored, &tool_cache, call)));
        }
        case_report(&built, &reports)
    }
}

fn case_report(
    built: &SemanticGuardrail<ScriptedRouter>,
    reports: &[Value],
) -> Result<Value, Box<HelperError>> {
    let hooks = match &built.base.settings.supported_event_hooks {
        Some(hooks) => serde_json::to_value(hooks)?,
        None => Value::Null,
    };
    Ok(json!({
        "provider": GUARDRAIL_PROVIDER,
        "embedding_model": built.embedding_model,
        "similarity_threshold": built.similarity_threshold,
        "on_flagged_action": built.on_flagged_action,
        "route_count": built.route_count,
        "default_on": built.base.settings.default_on,
        "supported_event_hooks": hooks,
        "calls": reports,
    }))
}

fn guardrail_args(settings: &Settings) -> SemanticGuardArgs {
    SemanticGuardArgs {
        name: settings.guardrail_name.clone(),
        embedding_model: settings.embedding_model.clone(),
        similarity_threshold: settings.similarity_threshold,
        route_templates: Vec::new(),
        custom_routes_file: None,
        custom_routes: settings.custom_routes.clone(),
        on_flagged_action: settings.on_flagged_action.clone(),
        event_hook: None,
        default_on: settings.default_on,
    }
}

async fn one_call(
    guardrail: &SemanticGuardrail<ScriptedRouter>,
    matches: &Mutex<bool>,
    scored: &Mutex<Vec<String>>,
    tool_cache: &DualLocalCache,
    call: &Call,
) -> Value {
    *matches.lock().await = call.matches;
    scored.lock().await.clear();
    let outcome = match &call.response {
        None => guardrail
            .async_pre_call_hook(&call.request, &call.call_type, tool_cache)
            .await
            .map(|()| None),
        Some(response) => guardrail
            .async_post_call_success_hook(&call.request, response.clone())
            .await
            .map(Some),
    };
    let texts = scored.lock().await.clone();
    call_report(&outcome, &texts)
}

fn call_report(
    outcome: &Result<Option<GuardrailValue>, GuardrailFailure>,
    scored: &[String],
) -> Value {
    match outcome {
        Ok(returned) => passed_report(returned.as_ref(), scored),
        Err(GuardrailFailure::HttpObject { status, detail }) => {
            blocked_report(*status, detail, scored)
        }
        Err(GuardrailFailure::ModifyResponse(failure)) => passthrough_report(failure, scored),
        Err(GuardrailFailure::Http { .. })
        | Err(GuardrailFailure::GuardrailRaised { .. })
        | Err(GuardrailFailure::BlockedPii { .. })
        | Err(GuardrailFailure::SensitiveDataRoute(_))
        | Err(GuardrailFailure::Other(_)) => error_report(scored),
    }
}

fn passed_report(returned: Option<&GuardrailValue>, scored: &[String]) -> Value {
    json!({
        "scored": scored,
        "kind": "passed",
        "status": null,
        "detail": null,
        "message": null,
        "model": null,
        "guardrail_name": null,
        "request_data": null,
        "detection_info": null,
        "returned": returned,
    })
}

fn blocked_report(
    status: u16,
    detail: &IndexMap<String, GuardrailValue>,
    scored: &[String],
) -> Value {
    json!({
        "scored": scored,
        "kind": "blocked",
        "status": status,
        "detail": detail,
        "message": null,
        "model": null,
        "guardrail_name": null,
        "request_data": null,
        "detection_info": null,
        "returned": null,
    })
}

fn passthrough_report(failure: &ModifyResponseFailure, scored: &[String]) -> Value {
    json!({
        "scored": scored,
        "kind": "passthrough",
        "status": null,
        "detail": null,
        "message": failure.message,
        "model": failure.model,
        "guardrail_name": failure.guardrail_name,
        "request_data": failure.request_data,
        "detection_info": failure.detection_info,
        "returned": null,
    })
}

fn error_report(scored: &[String]) -> Value {
    json!({
        "scored": scored,
        "kind": "error",
        "status": null,
        "detail": null,
        "message": null,
        "model": null,
        "guardrail_name": null,
        "request_data": null,
        "detection_info": null,
        "returned": null,
    })
}

fn cache() -> Result<DualLocalCache, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| failure("RedactionError", &error))?;
    Ok(DualLocalCache::new(
        None,
        None,
        &CacheSettings::default(),
        Arc::new(|| 0.0_f64),
        Diagnostics {
            redactor: Arc::new(redactor),
            emit: Arc::new(|_event| {}),
        },
    ))
}

fn failure(class: &str, error: &dyn std::error::Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
