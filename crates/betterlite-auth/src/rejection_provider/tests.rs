#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::resolution::{ProviderIdentity, ResolutionError, ResolvedModel};
use rstest::rstest;
use tracing::Subscriber;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use super::{
    DeploymentSnapshot, PROXY_LLM_PROVIDER_FALLBACK, ProviderResolution, SnapshotError,
    resolve_llm_provider_for_rate_limit,
};

enum Step {
    Success {
        model: String,
        provider: ProviderIdentity,
    },
    Failure,
}

#[derive(Default)]
struct ScriptedResolver {
    outcomes: Mutex<BTreeMap<String, VecDeque<Step>>>,
    calls: Mutex<Vec<String>>,
}

impl ScriptedResolver {
    fn script(&self, model: &str, steps: Vec<Step>) {
        self.outcomes
            .lock()
            .unwrap()
            .insert(model.to_owned(), VecDeque::from(steps));
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl ProviderResolution for ScriptedResolver {
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
            Some(Step::Success {
                model: resolved,
                provider,
            }) => Ok(ResolvedModel {
                model: resolved,
                provider,
                api_base: None,
                api_key: None,
            }),
            Some(Step::Failure) | None => Err(ResolutionError::MissingProvider {
                model: model.to_owned(),
            }),
        }
    }
}

enum SnapshotSource {
    Missing,
    Empty,
    Rows(Vec<Json>),
    Unreadable,
}

struct FixedSnapshot {
    source: SnapshotSource,
    reads: AtomicUsize,
}

impl DeploymentSnapshot for FixedSnapshot {
    fn model_list(&self) -> Result<Option<&[Json]>, SnapshotError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        match &self.source {
            SnapshotSource::Missing => Ok(None),
            SnapshotSource::Empty => Ok(Some(&[])),
            SnapshotSource::Rows(rows) => Ok(Some(rows.as_slice())),
            SnapshotSource::Unreadable => Err(SnapshotError::Unreadable),
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

fn observed<T, F>(run: F) -> (T, usize)
where
    F: FnOnce() -> T,
{
    let count = DebugCount::default();
    let subscriber = tracing_subscriber::registry().with(count.clone());
    let value = with_default(subscriber, run);
    (value, count.0.load(Ordering::Relaxed))
}

fn row(model_name: Json, params: Json) -> Json {
    Json::Object(JsonObject::from([
        ("model_name".to_owned(), model_name),
        ("litellm_params".to_owned(), params),
    ]))
}

fn alias_row(underlying: &str) -> Json {
    row(
        Json::Text("tpm-locked".to_owned()),
        params(Json::Text(underlying.to_owned())),
    )
}

fn openai(model: &str) -> Step {
    Step::Success {
        model: model.to_owned(),
        provider: ProviderIdentity::Known(ProviderId::Openai),
    }
}

fn params(model: Json) -> Json {
    Json::Object(JsonObject::from([("model".to_owned(), model)]))
}

#[rstest]
#[case::none(None)]
#[case::empty(Some(""))]
fn absent_model_no_lookup(#[case] model: Option<&str>) {
    let resolver = ScriptedResolver::default();
    resolver.script(
        "tpm-locked",
        vec![Step::Success {
            model: "gpt-4o-mini".to_owned(),
            provider: ProviderIdentity::Known(ProviderId::Openai),
        }],
    );
    let deployments = FixedSnapshot {
        source: SnapshotSource::Rows(vec![alias_row("openai/gpt-4o-mini")]),
        reads: AtomicUsize::new(0),
    };
    let (result, debug_events) =
        observed(|| resolve_llm_provider_for_rate_limit(model, &resolver, &deployments));
    assert_eq!(
        result,
        (String::new(), PROXY_LLM_PROVIDER_FALLBACK.to_owned())
    );
    assert!(resolver.calls().is_empty());
    assert_eq!(deployments.reads.load(Ordering::Relaxed), 0);
    assert_eq!(debug_events, 0);
}

#[test]
fn direct_resolution() {
    let resolver = ScriptedResolver::default();
    resolver.script(
        "anthropic/claude-3-5-sonnet",
        vec![Step::Success {
            model: "claude-3-5-sonnet".to_owned(),
            provider: ProviderIdentity::Known(ProviderId::Anthropic),
        }],
    );
    let deployments = FixedSnapshot {
        source: SnapshotSource::Rows(vec![row(
            Json::Text("anthropic/claude-3-5-sonnet".to_owned()),
            params(Json::Text("openai/gpt-4o-mini".to_owned())),
        )]),
        reads: AtomicUsize::new(0),
    };
    let (result, debug_events) = observed(|| {
        resolve_llm_provider_for_rate_limit(
            Some("anthropic/claude-3-5-sonnet"),
            &resolver,
            &deployments,
        )
    });
    assert_eq!(
        result,
        ("claude-3-5-sonnet".to_owned(), "anthropic".to_owned())
    );
    assert_eq!(
        resolver.calls(),
        vec!["anthropic/claude-3-5-sonnet".to_owned()]
    );
    assert_eq!(deployments.reads.load(Ordering::Relaxed), 0);
    assert_eq!(debug_events, 0);
}

#[rstest]
#[case::both_empty(
    "",
    ProviderIdentity::Custom(String::new()),
    "gpt-4o-mini",
    PROXY_LLM_PROVIDER_FALLBACK
)]
#[case::empty_model(
    "",
    ProviderIdentity::Known(ProviderId::Openai),
    "gpt-4o-mini",
    "openai"
)]
#[case::empty_provider(
    "gpt-4o-mini",
    ProviderIdentity::Custom(String::new()),
    "gpt-4o-mini",
    PROXY_LLM_PROVIDER_FALLBACK
)]
fn direct_empty_fields_no_alias(
    #[case] resolved_model: &str,
    #[case] provider: ProviderIdentity,
    #[case] expected_model: &str,
    #[case] expected_provider: &str,
) {
    let resolver = ScriptedResolver::default();
    resolver.script(
        "gpt-4o-mini",
        vec![Step::Success {
            model: resolved_model.to_owned(),
            provider,
        }],
    );
    let deployments = FixedSnapshot {
        source: SnapshotSource::Rows(vec![row(
            Json::Text("gpt-4o-mini".to_owned()),
            params(Json::Text("openai/other".to_owned())),
        )]),
        reads: AtomicUsize::new(0),
    };
    let (result, debug_events) = observed(|| {
        resolve_llm_provider_for_rate_limit(Some("gpt-4o-mini"), &resolver, &deployments)
    });
    assert_eq!(
        result,
        (expected_model.to_owned(), expected_provider.to_owned())
    );
    assert_eq!(resolver.calls(), vec!["gpt-4o-mini".to_owned()]);
    assert_eq!(deployments.reads.load(Ordering::Relaxed), 0);
    assert_eq!(debug_events, 0);
}

#[rstest]
#[case::resolved_model("gpt-4o-mini", "gpt-4o-mini")]
#[case::empty_resolved("", "openai/gpt-4o-mini")]
fn first_alias_success(#[case] returned: &str, #[case] expected_model: &str) {
    let resolver = ScriptedResolver::default();
    resolver.script("tpm-locked", vec![Step::Failure]);
    resolver.script(
        "openai/gpt-4o-mini",
        vec![Step::Success {
            model: returned.to_owned(),
            provider: ProviderIdentity::Known(ProviderId::Openai),
        }],
    );
    let deployments = FixedSnapshot {
        source: SnapshotSource::Rows(vec![alias_row("openai/gpt-4o-mini")]),
        reads: AtomicUsize::new(0),
    };
    let (result, debug_events) = observed(|| {
        resolve_llm_provider_for_rate_limit(Some("tpm-locked"), &resolver, &deployments)
    });
    assert_eq!(result, (expected_model.to_owned(), "openai".to_owned()));
    assert_eq!(
        resolver.calls(),
        vec!["tpm-locked".to_owned(), "openai/gpt-4o-mini".to_owned()]
    );
    assert_eq!(debug_events, 0);
}

fn malformed_rows() -> Vec<Json> {
    vec![
        Json::Text("not-a-deployment".to_owned()),
        Json::Number(serde_json::Number::from(7_u8)),
        Json::Null,
        Json::Object(JsonObject::from([(
            "model_name".to_owned(),
            Json::Number(serde_json::Number::from(7_u8)),
        )])),
        row(
            Json::Text("tpm-locked".to_owned()),
            Json::Text("openai/params-not-an-object".to_owned()),
        ),
        Json::Object(JsonObject::from([(
            "model_name".to_owned(),
            Json::Text("tpm-locked".to_owned()),
        )])),
        row(
            Json::Text("tpm-locked".to_owned()),
            Json::Object(JsonObject::new()),
        ),
        row(
            Json::Text("tpm-locked".to_owned()),
            params(Json::Number(serde_json::Number::from(3_u8))),
        ),
        row(
            Json::Text("tpm-locked".to_owned()),
            params(Json::Text(String::new())),
        ),
        row(
            Json::Text("other".to_owned()),
            params(Json::Text("openai/other".to_owned())),
        ),
        alias_row("openai/gpt-4o-mini"),
    ]
}

#[test]
fn malformed_deployments_skipped() {
    let resolver = ScriptedResolver::default();
    resolver.script("tpm-locked", vec![Step::Failure]);
    resolver.script("openai/gpt-4o-mini", vec![openai("gpt-4o-mini")]);
    let deployments = FixedSnapshot {
        source: SnapshotSource::Rows(malformed_rows()),
        reads: AtomicUsize::new(0),
    };
    let (result, debug_events) = observed(|| {
        resolve_llm_provider_for_rate_limit(Some("tpm-locked"), &resolver, &deployments)
    });
    assert_eq!(result, ("gpt-4o-mini".to_owned(), "openai".to_owned()));
    assert_eq!(
        resolver.calls(),
        vec!["tpm-locked".to_owned(), "openai/gpt-4o-mini".to_owned()]
    );
    assert_eq!(debug_events, 0);
}

#[test]
fn alias_failed_candidate_continues() {
    let resolver = ScriptedResolver::default();
    resolver.script("tpm-locked", vec![Step::Failure]);
    resolver.script("unknown/one", vec![Step::Failure]);
    resolver.script(
        "openai/gpt-4o-mini",
        vec![Step::Success {
            model: "gpt-4o-mini".to_owned(),
            provider: ProviderIdentity::Known(ProviderId::Openai),
        }],
    );
    let deployments = FixedSnapshot {
        source: SnapshotSource::Rows(vec![
            alias_row("unknown/one"),
            alias_row("openai/gpt-4o-mini"),
        ]),
        reads: AtomicUsize::new(0),
    };
    let (result, debug_events) = observed(|| {
        resolve_llm_provider_for_rate_limit(Some("tpm-locked"), &resolver, &deployments)
    });
    assert_eq!(result, ("gpt-4o-mini".to_owned(), "openai".to_owned()));
    assert_eq!(
        resolver.calls(),
        vec![
            "tpm-locked".to_owned(),
            "unknown/one".to_owned(),
            "openai/gpt-4o-mini".to_owned(),
        ]
    );
    assert_eq!(debug_events, 0);
}

#[test]
fn alias_missing_provider_continues() {
    let resolver = ScriptedResolver::default();
    resolver.script("tpm-locked", vec![Step::Failure]);
    resolver.script(
        "openai/anonymous",
        vec![Step::Success {
            model: String::new(),
            provider: ProviderIdentity::Custom(String::new()),
        }],
    );
    resolver.script("openai/gpt-4o-mini", vec![openai("gpt-4o-mini")]);
    let deployments = FixedSnapshot {
        source: SnapshotSource::Rows(vec![
            alias_row("openai/anonymous"),
            alias_row("openai/gpt-4o-mini"),
        ]),
        reads: AtomicUsize::new(0),
    };
    let (result, debug_events) = observed(|| {
        resolve_llm_provider_for_rate_limit(Some("tpm-locked"), &resolver, &deployments)
    });
    assert_eq!(result, ("gpt-4o-mini".to_owned(), "openai".to_owned()));
    assert_eq!(
        resolver.calls(),
        vec![
            "tpm-locked".to_owned(),
            "openai/anonymous".to_owned(),
            "openai/gpt-4o-mini".to_owned(),
        ]
    );
    assert_eq!(debug_events, 0);
}

#[rstest]
#[case::missing(SnapshotSource::Missing)]
#[case::empty(SnapshotSource::Empty)]
fn absent_router_or_list(#[case] source: SnapshotSource) {
    let resolver = ScriptedResolver::default();
    resolver.script("tpm-locked", vec![Step::Failure]);
    let deployments = FixedSnapshot {
        source,
        reads: AtomicUsize::new(0),
    };
    let (result, debug_events) = observed(|| {
        resolve_llm_provider_for_rate_limit(Some("tpm-locked"), &resolver, &deployments)
    });
    assert_eq!(
        result,
        (
            "tpm-locked".to_owned(),
            PROXY_LLM_PROVIDER_FALLBACK.to_owned()
        )
    );
    assert_eq!(resolver.calls(), vec!["tpm-locked".to_owned()]);
    assert_eq!(debug_events, 1);
}

#[test]
fn alias_outer_error_silent() {
    let resolver = ScriptedResolver::default();
    resolver.script("tpm-locked", vec![Step::Failure]);
    let deployments = FixedSnapshot {
        source: SnapshotSource::Unreadable,
        reads: AtomicUsize::new(0),
    };
    let (result, debug_events) = observed(|| {
        resolve_llm_provider_for_rate_limit(Some("tpm-locked"), &resolver, &deployments)
    });
    assert_eq!(
        result,
        (
            "tpm-locked".to_owned(),
            PROXY_LLM_PROVIDER_FALLBACK.to_owned()
        )
    );
    assert_eq!(resolver.calls(), vec!["tpm-locked".to_owned()]);
    assert_eq!(debug_events, 1);
}

#[test]
fn unresolved_keeps_original_model_debug_once() {
    let resolver = ScriptedResolver::default();
    resolver.script("tpm-locked", vec![Step::Failure]);
    let deployments = FixedSnapshot {
        source: SnapshotSource::Rows(vec![row(
            Json::Text("other".to_owned()),
            params(Json::Text("openai/gpt-4o-mini".to_owned())),
        )]),
        reads: AtomicUsize::new(0),
    };
    let (result, debug_events) = observed(|| {
        resolve_llm_provider_for_rate_limit(Some("tpm-locked"), &resolver, &deployments)
    });
    assert_eq!(
        result,
        (
            "tpm-locked".to_owned(),
            PROXY_LLM_PROVIDER_FALLBACK.to_owned()
        )
    );
    assert_eq!(resolver.calls(), vec!["tpm-locked".to_owned()]);
    assert_eq!(debug_events, 1);
}

#[test]
fn provider_failure_never_masks_budget() {
    let resolver = ScriptedResolver::default();
    resolver.script("tpm-locked", vec![Step::Failure]);
    resolver.script("openai/gpt-4o-mini", vec![Step::Failure]);
    let deployments = FixedSnapshot {
        source: SnapshotSource::Rows(vec![alias_row("openai/gpt-4o-mini")]),
        reads: AtomicUsize::new(0),
    };
    let (result, debug_events) = observed(|| {
        resolve_llm_provider_for_rate_limit(Some("tpm-locked"), &resolver, &deployments)
    });
    assert_eq!(
        result,
        (
            "tpm-locked".to_owned(),
            PROXY_LLM_PROVIDER_FALLBACK.to_owned()
        )
    );
    assert!(!result.0.is_empty());
    assert_eq!(
        resolver.calls(),
        vec!["tpm-locked".to_owned(), "openai/gpt-4o-mini".to_owned()]
    );
    assert_eq!(debug_events, 1);
}
