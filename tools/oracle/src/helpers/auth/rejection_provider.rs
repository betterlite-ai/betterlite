use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::rejection_provider::{DeploymentSnapshot, ProviderResolution, SnapshotError};
// ast-grep-ignore: no-long-names
use betterlite_auth::rejection_provider::resolve_llm_provider_for_rate_limit as attribute_rejection;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ProviderIdentity, ResolutionError, ResolvedModel};
use serde::Deserialize;
use tracing::Subscriber;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: Option<String>,
    router: RouterKind,
    deployments: Vec<Json>,
    resolutions: Vec<ResolutionStep>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RouterKind {
    Absent,
    Empty,
    Unreadable,
    Deployments,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
enum ResolutionStep {
    Success {
        model: String,
        resolved: String,
        provider: String,
    },
    Failure {
        model: String,
    },
}

enum Outcome {
    Success { model: String, provider: String },
    Failure,
}

struct ScriptedLookup {
    outcomes: Mutex<BTreeMap<String, VecDeque<Outcome>>>,
    calls: Mutex<Vec<String>>,
}

impl ProviderResolution for ScriptedLookup {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        let next = self
            .outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(model)
            .and_then(VecDeque::pop_front);
        match next {
            Some(Outcome::Success {
                model: resolved,
                provider,
            }) => Ok(ResolvedModel {
                model: resolved,
                // the scripted name is the resolved provider itself, so no identity table stands in for it
                provider: ProviderIdentity::Custom(provider),
                api_base: None,
                api_key: None,
            }),
            Some(Outcome::Failure) | None => Err(ResolutionError::MissingProvider {
                model: model.to_owned(),
            }),
        }
    }
}

struct FixtureSnapshot {
    router: RouterKind,
    deployments: Vec<Json>,
}

impl DeploymentSnapshot for FixtureSnapshot {
    fn model_list(&self) -> Result<Option<&[Json]>, SnapshotError> {
        match self.router {
            RouterKind::Absent => Ok(None),
            RouterKind::Empty => Ok(Some(&[])),
            RouterKind::Unreadable => Err(SnapshotError::Unreadable),
            RouterKind::Deployments => Ok(Some(&self.deployments)),
        }
    }
}

#[derive(Clone, Default)]
struct DebugCount(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for DebugCount {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        if event.metadata().level() == &tracing::Level::DEBUG {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn scripted(steps: &[ResolutionStep]) -> ScriptedLookup {
    let mut outcomes: BTreeMap<String, VecDeque<Outcome>> = BTreeMap::new();
    for step in steps {
        match step {
            ResolutionStep::Success {
                model,
                resolved,
                provider,
            } => outcomes
                .entry(model.clone())
                .or_default()
                .push_back(Outcome::Success {
                    model: resolved.clone(),
                    provider: provider.clone(),
                }),
            ResolutionStep::Failure { model } => outcomes
                .entry(model.clone())
                .or_default()
                .push_back(Outcome::Failure),
        }
    }
    ScriptedLookup {
        outcomes: Mutex::new(outcomes),
        calls: Mutex::new(Vec::new()),
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let lookup = scripted(&input.resolutions);
        let snapshot = FixtureSnapshot {
            router: input.router,
            deployments: input.deployments.clone(),
        };
        let count = DebugCount::default();
        let subscriber = tracing_subscriber::registry().with(count.clone());
        let (model, provider) = with_default(subscriber, || {
            attribute_rejection(input.model.as_deref(), &lookup, &snapshot)
        });
        let calls = lookup
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(serde_json::json!({
            "model": model,
            "provider": provider,
            "calls": calls,
            "debugs": count.0.load(Ordering::Relaxed),
        }))
    }
}
