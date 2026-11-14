#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use crate::pipeline::{GuardrailPipeline, PipelineAction, PipelineMode, PipelineStep};
use crate::validation::lookup::{AliasDatabase, GuardrailRegistry, ModelRouter};
use crate::values::{Policy, PolicyGuardrails};

#[derive(Debug, thiserror::Error)]
#[error("backend down")]
pub struct FakeFailure;

pub struct FakeRegistry {
    pub names: Option<Vec<String>>,
    pub calls: AtomicUsize,
}

impl GuardrailRegistry for FakeRegistry {
    type Error = FakeFailure;

    fn list_guardrail_names(&self) -> Result<Vec<String>, FakeFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.names.clone().ok_or(FakeFailure)
    }
}

pub struct FakeDatabase {
    pub teams: Vec<String>,
    pub keys: Vec<String>,
    pub failing: bool,
    pub queries: Mutex<Vec<String>>,
}

impl FakeDatabase {
    fn answer(&self, kind: &str, alias: &str, known: &[String]) -> Result<bool, FakeFailure> {
        self.queries.lock().unwrap().push(format!("{kind}:{alias}"));
        if self.failing {
            return Err(FakeFailure);
        }
        Ok(known.iter().any(|entry| entry == alias))
    }

    pub fn recorded(&self) -> Vec<String> {
        self.queries.lock().unwrap().clone()
    }
}

impl AliasDatabase for FakeDatabase {
    type Error = FakeFailure;

    fn team_alias_exists(
        &self,
        team_alias: &str,
    ) -> impl Future<Output = Result<bool, FakeFailure>> + Send {
        ready(self.answer("team", team_alias, &self.teams))
    }

    fn key_alias_exists(
        &self,
        key_alias: &str,
    ) -> impl Future<Output = Result<bool, FakeFailure>> + Send {
        ready(self.answer("key", key_alias, &self.keys))
    }
}

pub struct FakeRouter {
    pub model_names: Vec<String>,
    pub patterns: Vec<String>,
    pub failing_names: bool,
    pub failing_patterns: bool,
    pub pattern_calls: AtomicUsize,
}

impl ModelRouter for FakeRouter {
    type Error = FakeFailure;

    fn has_model_name(&self, model: &str) -> Result<bool, FakeFailure> {
        if self.failing_names {
            return Err(FakeFailure);
        }
        Ok(self.model_names.iter().any(|name| name == model))
    }

    fn has_pattern_deployments(&self, model: &str) -> Result<bool, FakeFailure> {
        self.pattern_calls.fetch_add(1, Ordering::SeqCst);
        if self.failing_patterns {
            return Err(FakeFailure);
        }
        Ok(self.patterns.iter().any(|pattern| pattern == model))
    }
}

pub fn texts(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

pub fn registry(names: &[&str]) -> FakeRegistry {
    FakeRegistry {
        names: Some(texts(names)),
        calls: AtomicUsize::new(0),
    }
}

pub fn database(teams: &[&str], keys: &[&str]) -> FakeDatabase {
    FakeDatabase {
        teams: texts(teams),
        keys: texts(keys),
        failing: false,
        queries: Mutex::new(Vec::new()),
    }
}

pub fn router(model_names: &[&str], patterns: &[&str]) -> FakeRouter {
    FakeRouter {
        model_names: texts(model_names),
        patterns: texts(patterns),
        failing_names: false,
        failing_patterns: false,
        pattern_calls: AtomicUsize::new(0),
    }
}

pub fn policy(inherit: Option<&str>, add: &[&str], remove: &[&str]) -> Policy {
    Policy {
        inherit: inherit.map(str::to_owned),
        description: None,
        guardrails: PolicyGuardrails {
            add: Some(texts(add)),
            remove: Some(texts(remove)),
        },
        condition: None,
        pipeline: None,
    }
}

pub fn pipeline_policy(add: &[&str], steps: &[&str]) -> Policy {
    let mut piped = policy(None, add, &[]);
    let steps = steps
        .iter()
        .map(|guardrail| PipelineStep {
            guardrail: (*guardrail).to_owned(),
            on_fail: PipelineAction::Block,
            on_pass: PipelineAction::Allow,
            on_error: None,
            pass_data: false,
            modify_response_message: None,
        })
        .collect();
    piped.pipeline = Some(GuardrailPipeline {
        mode: PipelineMode::PreCall,
        steps,
    });
    piped
}

pub fn policies(entries: Vec<(&str, Policy)>) -> BTreeMap<String, Policy> {
    entries
        .into_iter()
        .map(|(name, entry)| (name.to_owned(), entry))
        .collect()
}

struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

struct WarningCapture(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for WarningCapture {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if event.metadata().level() == &Level::WARN {
            let mut visitor = MessageVisitor(String::new());
            event.record(&mut visitor);
            self.0.lock().unwrap().push(visitor.0);
        }
    }
}

pub struct Warnings {
    messages: Arc<Mutex<Vec<String>>>,
    _guard: DefaultGuard,
}

impl Warnings {
    pub fn capture() -> Self {
        let messages = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(WarningCapture(Arc::clone(&messages)));
        Self {
            messages,
            _guard: set_default(subscriber),
        }
    }

    pub fn messages(&self) -> Vec<String> {
        self.messages.lock().unwrap().clone()
    }
}
