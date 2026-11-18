use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::local_ttl_cache::Settings as CacheSettings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::semantic_guard::semantic_guard_route_loading::builder::SemanticRouterBuilder;
use betterlite_guardrails::semantic_guard::semantic_guard_route_loading::route::Route;
use betterlite_guardrails::semantic_guard_hook::init::SemanticGuardArgs;
use betterlite_guardrails::semantic_guard_hook::scoring::{RouteChoice, SemanticRouteScorer};
use betterlite_guardrails::semantic_guard_hook::{
    GUARDRAIL_PROVIDER, SemanticGuardrail, load_routes,
};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
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
        let args = guardrail_args(&input.settings);
        let routes = load_routes(&args).or_value_error()?;
        let built = runtime
            .block_on(SemanticGuardrail::new(args, routes, &builder, true))
            .or_value_error()?;
        let tool_cache = cache()?;
        let mut reports = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            reports.push(runtime.block_on(one_call(
                &built,
                &matches,
                &scored,
                &tool_cache,
                call,
            ))?);
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
) -> Result<Value, Box<HelperError>> {
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

#[derive(Serialize)]
struct CallReport<'a> {
    scored: &'a [String],
    kind: &'a str,
    status: Option<u16>,
    detail: Option<&'a IndexMap<String, GuardrailValue>>,
    message: Option<&'a str>,
    model: Option<&'a str>,
    guardrail_name: Option<&'a str>,
    request_data: Option<&'a BTreeMap<String, GuardrailValue>>,
    detection_info: Option<&'a BTreeMap<String, GuardrailValue>>,
    returned: Option<&'a GuardrailValue>,
}

impl<'a> CallReport<'a> {
    fn empty(scored: &'a [String], kind: &'a str) -> Self {
        Self {
            scored,
            kind,
            status: None,
            detail: None,
            message: None,
            model: None,
            guardrail_name: None,
            request_data: None,
            detection_info: None,
            returned: None,
        }
    }
}

fn call_report(
    outcome: &Result<Option<GuardrailValue>, GuardrailFailure>,
    scored: &[String],
) -> Result<Value, Box<HelperError>> {
    let report = match outcome {
        Ok(returned) => {
            let mut report = CallReport::empty(scored, "passed");
            report.returned = returned.as_ref();
            report
        }
        Err(GuardrailFailure::HttpObject { status, detail }) => {
            let mut report = CallReport::empty(scored, "blocked");
            report.status = Some(*status);
            report.detail = Some(detail);
            report
        }
        Err(GuardrailFailure::ModifyResponse(failure)) => {
            let mut report = CallReport::empty(scored, "passthrough");
            report.message = Some(failure.message.as_str());
            report.model = Some(failure.model.as_str());
            report.guardrail_name = failure.guardrail_name.as_deref();
            report.request_data = Some(&failure.request_data);
            report.detection_info = Some(&failure.detection_info);
            report
        }
        Err(GuardrailFailure::Http { .. })
        | Err(GuardrailFailure::GuardrailRaised { .. })
        | Err(GuardrailFailure::BlockedPii { .. })
        | Err(GuardrailFailure::SensitiveDataRoute(_))
        | Err(GuardrailFailure::Other(_)) => CallReport::empty(scored, "error"),
    };
    Ok(serde_json::to_value(&report)?)
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
