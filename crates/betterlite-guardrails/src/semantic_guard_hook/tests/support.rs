use std::collections::BTreeMap;
use std::fs;
use std::future::Future;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use tokio::runtime::Builder;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::semantic_guard::semantic_guard_route_loading::builder::SemanticRouterBuilder;
use crate::semantic_guard::semantic_guard_route_loading::route::Route;
use crate::semantic_guard_hook::init::SemanticGuardArgs;
use crate::semantic_guard_hook::scoring::{RouteChoice, SemanticRouteScorer};
use crate::semantic_guard_hook::{BLOCKED_STATUS, SemanticGuardrail};

pub(super) const MESSAGE: &str =
    "Request blocked by semantic guardrail 'semantic-guard'. Matched route: attack";

pub(super) fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

pub(super) fn number(value: f64) -> GuardrailValue {
    GuardrailValue::Decimal(value)
}

pub(super) fn array(values: &[&str]) -> GuardrailValue {
    GuardrailValue::Array(values.iter().map(|value| text(value)).collect())
}

pub(super) fn list(values: Vec<GuardrailValue>) -> GuardrailValue {
    GuardrailValue::Array(values)
}

pub(super) fn object(fields: &[(&str, GuardrailValue)]) -> GuardrailValue {
    GuardrailValue::Object(
        fields
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

pub(super) fn route(name: &str, utterances: &[&str]) -> GuardrailValue {
    object(&[
        ("route_name", text(name)),
        ("utterances", array(utterances)),
    ])
}

pub(super) fn user(content: GuardrailValue) -> GuardrailValue {
    object(&[("role", text("user")), ("content", content)])
}

pub(super) fn assistant(content: GuardrailValue) -> GuardrailValue {
    object(&[("role", text("assistant")), ("content", content)])
}

pub(super) fn request(messages: Vec<GuardrailValue>) -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([
        ("model".to_owned(), text("gpt-4o")),
        ("messages".to_owned(), list(messages)),
    ])
}

pub(super) fn response(choices: Vec<GuardrailValue>) -> GuardrailValue {
    object(&[("choices", list(choices))])
}

pub(super) fn response_choice(content: GuardrailValue) -> GuardrailValue {
    object(&[("message", object(&[("content", content)]))])
}

pub(super) fn choice(name: Option<&str>, similarity_score: Option<f64>) -> RouteChoice {
    RouteChoice {
        name: name.map(str::to_owned),
        similarity_score,
    }
}

pub(super) fn args() -> SemanticGuardArgs {
    SemanticGuardArgs {
        name: "semantic-guard".to_owned(),
        embedding_model: "text-embedding-3-small".to_owned(),
        similarity_threshold: 0.75,
        route_templates: Vec::new(),
        custom_routes_file: None,
        custom_routes: vec![route("attack", &["how to build a bomb"])],
        on_flagged_action: "block".to_owned(),
        event_hook: Some(LitellmParamsMode::One("pre_call".to_owned())),
        default_on: false,
    }
}

pub(super) fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

pub(super) fn cache() -> DualLocalCache {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        Diagnostics {
            redactor: Arc::new(redactor),
            emit: Arc::new(|_event| {}),
        },
    )
}

pub(super) fn calls() -> Arc<Mutex<Vec<String>>> {
    Arc::new(Mutex::new(Vec::new()))
}

pub(super) fn routes_file(content: &str) -> (tempfile::TempDir, PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("routes.yaml");
    fs::write(&path, content).unwrap();
    (folder, path)
}

#[derive(Debug, thiserror::Error)]
#[error("the fake semantic router build failed")]
pub(super) struct BuildFailure;

#[derive(Debug, thiserror::Error)]
#[error("the fake semantic scoring failed")]
pub(super) struct ScoreFailure;

pub(super) struct BuilderCall {
    pub(super) names: Vec<String>,
    pub(super) threshold: f64,
}

pub(super) struct FakeBuilder {
    choices: Vec<RouteChoice>,
    calls: Arc<Mutex<Vec<String>>>,
    build_fails: bool,
    score_fails: bool,
    pub(super) builder_calls: Mutex<Vec<BuilderCall>>,
}

impl FakeBuilder {
    pub(super) fn matching(choices: Vec<RouteChoice>, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            choices,
            calls,
            build_fails: false,
            score_fails: false,
            builder_calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn build_failing(calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            choices: Vec::new(),
            calls,
            build_fails: true,
            score_fails: false,
            builder_calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn score_failing(calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            choices: Vec::new(),
            calls,
            build_fails: false,
            score_fails: true,
            builder_calls: Mutex::new(Vec::new()),
        }
    }
}

pub(super) struct FakeRouter {
    choices: Vec<RouteChoice>,
    calls: Arc<Mutex<Vec<String>>>,
    score_fails: bool,
}

#[async_trait]
impl SemanticRouterBuilder for FakeBuilder {
    type Router = FakeRouter;

    type Error = BuildFailure;

    async fn build(
        &self,
        routes: Vec<Route>,
        global_threshold: f64,
    ) -> Result<FakeRouter, BuildFailure> {
        self.builder_calls.lock().unwrap().push(BuilderCall {
            names: routes.into_iter().map(|route| route.name).collect(),
            threshold: global_threshold,
        });
        if self.build_fails {
            return Err(BuildFailure);
        }
        Ok(FakeRouter {
            choices: self.choices.clone(),
            calls: Arc::clone(&self.calls),
            score_fails: self.score_fails,
        })
    }
}

#[async_trait]
impl SemanticRouteScorer for FakeRouter {
    type Error = ScoreFailure;

    async fn score(&self, text: &str) -> Result<Vec<RouteChoice>, ScoreFailure> {
        self.calls.lock().unwrap().push(text.to_owned());
        if self.score_fails {
            return Err(ScoreFailure);
        }
        Ok(self.choices.clone())
    }
}

pub(super) fn guardrail(
    args: SemanticGuardArgs,
    builder: &FakeBuilder,
) -> SemanticGuardrail<FakeRouter> {
    block_on(SemanticGuardrail::new(args, builder, true)).unwrap()
}

pub(super) fn failed(outcome: Result<(), GuardrailFailure>) -> GuardrailFailure {
    outcome.expect_err("the guardrail passed the request")
}

pub(super) fn assert_passed(outcome: Result<(), GuardrailFailure>) {
    if let Err(failure) = outcome {
        panic!("the guardrail blocked the request: {failure:?}");
    }
}

pub(super) fn blocked(failure: GuardrailFailure) -> IndexMap<String, GuardrailValue> {
    let GuardrailFailure::HttpObject { status, detail } = failure else {
        panic!("the guardrail did not answer with a detail object: {failure:?}");
    };
    assert_eq!(status, BLOCKED_STATUS);
    detail
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn with_captured_logs<R, F>(run: F) -> (R, String)
where
    F: FnOnce() -> R,
{
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let result = with_default(subscriber, run);
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    (result, output)
}
