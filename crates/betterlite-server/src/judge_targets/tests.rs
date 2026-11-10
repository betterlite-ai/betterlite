#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::judge_primitives::provider_qualified::ProviderCache;
use betterlite_models::judge_primitives::target::{JudgeTarget, JudgeVia};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use indexmap::IndexMap;
use serde_json::Number;

use super::call::JudgeCall;
use super::dispatcher::JudgeDispatcher;
use super::error::{JudgeCallError, JudgeDispatchError};
use super::router::JudgeRouter;
use super::sdk::JudgeSdk;
use super::{judge_acompletion, judge_target};

const CATALOG: &str = r#"{"gpt-4o": {"litellm_provider": "openai"}}"#;

struct Served {
    model: &'static str,
    team_id: Option<&'static str>,
    models: Vec<&'static str>,
}

struct RouterFake {
    served: Vec<Served>,
    failure: Option<&'static str>,
    calls: Mutex<Vec<JudgeCall>>,
}

impl RouterFake {
    fn new(served: Vec<Served>) -> Self {
        Self {
            served,
            failure: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            served: vec![Served {
                model: "judge",
                team_id: None,
                models: vec!["openai/gpt-4o"],
            }],
            failure: Some("provider exploded"),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<JudgeCall> {
        call_log(&self.calls)
    }
}

#[async_trait]
impl JudgeRouter for RouterFake {
    fn resolved_litellm_models(&self, model: &str, team_id: Option<&str>) -> Vec<String> {
        self.served
            .iter()
            .filter(|entry| {
                entry.model == model && entry.team_id.is_none_or(|team| Some(team) == team_id)
            })
            .flat_map(|entry| entry.models.iter().map(|served| (*served).to_owned()))
            .collect()
    }

    async fn acompletion(&self, call: JudgeCall) -> Result<Json, JudgeDispatchError> {
        record_call(&self.calls, self.failure, call)
    }
}

#[derive(Default)]
struct SdkFake {
    failure: Option<&'static str>,
    calls: Mutex<Vec<JudgeCall>>,
}

impl SdkFake {
    fn calls(&self) -> Vec<JudgeCall> {
        call_log(&self.calls)
    }
}

#[async_trait]
impl JudgeSdk for SdkFake {
    async fn acompletion(&self, call: JudgeCall) -> Result<Json, JudgeDispatchError> {
        record_call(&self.calls, self.failure, call)
    }
}

fn record_call(
    calls: &Mutex<Vec<JudgeCall>>,
    failure: Option<&'static str>,
    call: JudgeCall,
) -> Result<Json, JudgeDispatchError> {
    if let Some(message) = failure {
        return Err(JudgeDispatchError {
            message: message.to_owned(),
        });
    }
    calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(call);
    Ok(Json::Null)
}

fn call_log(calls: &Mutex<Vec<JudgeCall>>) -> Vec<JudgeCall> {
    calls.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

fn context<'context>(
    catalog: &'context ModelCatalog,
    known: &'context BTreeSet<String>,
    environment: &'context BTreeMap<String, String>,
    custom: &'context BTreeSet<String>,
) -> ResolutionContext<'context> {
    ResolutionContext {
        catalog,
        known_model_names: known,
        custom_providers: custom,
        environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    }
}

fn message() -> Vec<Json> {
    vec![Json::Object(IndexMap::from([
        ("role".to_owned(), Json::Text("user".to_owned())),
        (
            "content".to_owned(),
            Json::Text("which reply is better?".to_owned()),
        ),
    ]))]
}

fn params() -> IndexMap<String, Json> {
    IndexMap::from([("temperature".to_owned(), Json::Number(Number::from(0)))])
}

fn targets(models: &[&str]) -> BTreeSet<String> {
    models.iter().map(|model| (*model).to_owned()).collect()
}

#[test]
fn judge_target_records_the_dispatch_path_and_the_resolved_models() {
    let target = JudgeTarget {
        via: JudgeVia::Router,
        models: targets(&["openai/gpt-4o"]),
    };

    assert_eq!(serde_json::to_value(target.via).unwrap(), "router");
    assert_eq!(serde_json::to_value(JudgeVia::Sdk).unwrap(), "sdk");
    assert_eq!(serde_json::to_value(JudgeVia::Nothing).unwrap(), "nothing");
    assert_eq!(target.models, targets(&["openai/gpt-4o"]));
}

#[test]
fn judge_target_takes_the_router_path_when_the_router_serves_the_model() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap();
    let (known, environment, custom) = (BTreeSet::new(), BTreeMap::new(), BTreeSet::new());
    let context = context(&catalog, &known, &environment, &custom);
    let cache = ProviderCache::default();
    let router = RouterFake::new(vec![Served {
        model: "judge",
        team_id: None,
        models: vec!["gpt-4o", "unmapped-deployment"],
    }]);
    let sdk = SdkFake::default();
    let dispatcher = JudgeDispatcher {
        router: Some(&router),
        sdk: &sdk,
        cache: &cache,
        context: &context,
    };

    let target = judge_target(&dispatcher, "judge", None);

    assert_eq!(target.via, JudgeVia::Router);
    assert_eq!(
        target.models,
        targets(&["openai/gpt-4o", "unmapped-deployment"])
    );
}

#[test]
fn judge_target_takes_the_sdk_path_for_a_public_provider_name() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap();
    let (known, environment, custom) = (BTreeSet::new(), BTreeMap::new(), BTreeSet::new());
    let context = context(&catalog, &known, &environment, &custom);
    let cache = ProviderCache::default();
    let sdk = SdkFake::default();
    let dispatcher = JudgeDispatcher {
        router: None,
        sdk: &sdk,
        cache: &cache,
        context: &context,
    };

    let target = judge_target(&dispatcher, "gpt-4o", None);

    assert_eq!(target.via, JudgeVia::Sdk);
    assert_eq!(target.models, targets(&["openai/gpt-4o"]));
}

#[test]
fn judge_target_reports_nothing_when_no_path_resolves() {
    let catalog = ModelCatalog::default();
    let (known, environment, custom) = (BTreeSet::new(), BTreeMap::new(), BTreeSet::new());
    let context = context(&catalog, &known, &environment, &custom);
    let cache = ProviderCache::default();
    let router = RouterFake::new(Vec::new());
    let sdk = SdkFake::default();
    let dispatcher = JudgeDispatcher {
        router: Some(&router),
        sdk: &sdk,
        cache: &cache,
        context: &context,
    };

    let target = judge_target(&dispatcher, "brand-new-unknown", None);

    assert_eq!(target.via, JudgeVia::Nothing);
    assert!(target.models.is_empty());
}

#[test]
fn judge_target_asks_the_router_for_the_callers_team() {
    let catalog = ModelCatalog::default();
    let (known, environment, custom) = (BTreeSet::new(), BTreeMap::new(), BTreeSet::new());
    let context = context(&catalog, &known, &environment, &custom);
    let cache = ProviderCache::default();
    let router = RouterFake::new(vec![
        Served {
            model: "team-judge",
            team_id: Some("team-a"),
            models: vec!["openai/gpt-4o"],
        },
        Served {
            model: "public-judge",
            team_id: None,
            models: vec!["openai/gpt-4o-mini"],
        },
    ]);
    let sdk = SdkFake::default();
    let dispatcher = JudgeDispatcher {
        router: Some(&router),
        sdk: &sdk,
        cache: &cache,
        context: &context,
    };

    let own_team = judge_target(&dispatcher, "team-judge", Some("team-a"));
    let no_team = judge_target(&dispatcher, "team-judge", None);
    let other_team = judge_target(&dispatcher, "team-judge", Some("team-b"));
    let public = judge_target(&dispatcher, "public-judge", Some("team-b"));

    assert_eq!(own_team.via, JudgeVia::Router);
    assert_eq!(own_team.models, targets(&["openai/gpt-4o"]));
    assert_eq!(no_team.via, JudgeVia::Nothing);
    assert_eq!(other_team.via, JudgeVia::Nothing);
    assert_eq!(public.via, JudgeVia::Router);
    assert_eq!(public.models, targets(&["openai/gpt-4o-mini"]));
}

#[tokio::test]
async fn judge_acompletion_dispatches_through_the_router_without_retries_or_fallbacks() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap();
    let (known, environment, custom) = (BTreeSet::new(), BTreeMap::new(), BTreeSet::new());
    let context = context(&catalog, &known, &environment, &custom);
    let cache = ProviderCache::default();
    let router = RouterFake::new(vec![Served {
        model: "judge",
        team_id: None,
        models: vec!["openai/gpt-4o"],
    }]);
    let sdk = SdkFake::default();
    let dispatcher = JudgeDispatcher {
        router: Some(&router),
        sdk: &sdk,
        cache: &cache,
        context: &context,
    };

    let response = judge_acompletion(&dispatcher, "judge", message(), None, params())
        .await
        .unwrap();

    assert_eq!(response, Json::Null);
    let calls = router.calls();
    let call = calls.first().expect("the router was called");
    assert_eq!(calls.len(), 1);
    assert_eq!(call.model, "judge");
    assert_eq!(call.messages, message());
    assert_eq!(call.num_retries, 0);
    assert_eq!(call.fallbacks, Some(Vec::new()));
    assert!(call.drop_params);
    assert_eq!(call.params, params());
    assert!(sdk.calls().is_empty());
}

#[tokio::test]
async fn judge_acompletion_dispatches_through_the_sdk_without_retries() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap();
    let (known, environment, custom) = (BTreeSet::new(), BTreeMap::new(), BTreeSet::new());
    let context = context(&catalog, &known, &environment, &custom);
    let cache = ProviderCache::default();
    let sdk = SdkFake::default();
    let dispatcher = JudgeDispatcher {
        router: None,
        sdk: &sdk,
        cache: &cache,
        context: &context,
    };

    let response = judge_acompletion(&dispatcher, "gpt-4o", message(), None, params())
        .await
        .unwrap();

    assert_eq!(response, Json::Null);
    let calls = sdk.calls();
    let call = calls.first().expect("the sdk was called");
    assert_eq!(calls.len(), 1);
    assert_eq!(call.model, "gpt-4o");
    assert_eq!(call.messages, message());
    assert_eq!(call.num_retries, 0);
    assert_eq!(call.fallbacks, None);
    assert!(call.drop_params);
    assert_eq!(call.params, params());
}

#[tokio::test]
async fn judge_acompletion_declines_a_model_that_resolves_to_nothing() {
    let catalog = ModelCatalog::default();
    let (known, environment, custom) = (BTreeSet::new(), BTreeMap::new(), BTreeSet::new());
    let context = context(&catalog, &known, &environment, &custom);
    let cache = ProviderCache::default();
    let sdk = SdkFake::default();
    let dispatcher = JudgeDispatcher {
        router: None,
        sdk: &sdk,
        cache: &cache,
        context: &context,
    };

    let result =
        judge_acompletion(&dispatcher, "brand-new-unknown", message(), None, params()).await;

    assert!(matches!(result, Err(JudgeCallError::Nothing { .. })));
    assert!(sdk.calls().is_empty());
}

#[tokio::test]
async fn judge_acompletion_resolves_the_target_under_the_callers_team() {
    let catalog = ModelCatalog::default();
    let (known, environment, custom) = (BTreeSet::new(), BTreeMap::new(), BTreeSet::new());
    let context = context(&catalog, &known, &environment, &custom);
    let cache = ProviderCache::default();
    let router = RouterFake::new(vec![Served {
        model: "team-judge",
        team_id: Some("team-a"),
        models: vec!["openai/gpt-4o"],
    }]);
    let sdk = SdkFake::default();
    let dispatcher = JudgeDispatcher {
        router: Some(&router),
        sdk: &sdk,
        cache: &cache,
        context: &context,
    };

    let without_team =
        judge_acompletion(&dispatcher, "team-judge", message(), None, params()).await;
    let with_team = judge_acompletion(
        &dispatcher,
        "team-judge",
        message(),
        Some("team-a"),
        params(),
    )
    .await;

    assert!(matches!(without_team, Err(JudgeCallError::Nothing { .. })));
    assert_eq!(with_team.unwrap(), Json::Null);
    assert_eq!(router.calls().len(), 1);
    assert!(sdk.calls().is_empty());
}

#[tokio::test]
async fn judge_acompletion_reports_a_backend_failure() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap();
    let (known, environment, custom) = (BTreeSet::new(), BTreeMap::new(), BTreeSet::new());
    let context = context(&catalog, &known, &environment, &custom);
    let cache = ProviderCache::default();
    let router = RouterFake::failing();
    let sdk = SdkFake::default();
    let dispatcher = JudgeDispatcher {
        router: Some(&router),
        sdk: &sdk,
        cache: &cache,
        context: &context,
    };

    let result = judge_acompletion(&dispatcher, "judge", message(), None, params()).await;

    match result {
        Err(JudgeCallError::Dispatch { model, source }) => {
            assert_eq!(model, "judge");
            assert_eq!(source.message, "provider exploded");
        }
        other => panic!("expected a dispatch failure, got {other:?}"),
    }
}

#[tokio::test]
async fn judge_acompletion_reports_an_sdk_backend_failure() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap();
    let (known, environment, custom) = (BTreeSet::new(), BTreeMap::new(), BTreeSet::new());
    let context = context(&catalog, &known, &environment, &custom);
    let cache = ProviderCache::default();
    let sdk = SdkFake {
        failure: Some("sdk exploded"),
        calls: Mutex::new(Vec::new()),
    };
    let dispatcher = JudgeDispatcher {
        router: None,
        sdk: &sdk,
        cache: &cache,
        context: &context,
    };

    let result = judge_acompletion(&dispatcher, "gpt-4o", message(), None, params()).await;

    match result {
        Err(JudgeCallError::Dispatch { model, source }) => {
            assert_eq!(model, "gpt-4o");
            assert_eq!(source.message, "sdk exploded");
        }
        other => panic!("expected a dispatch failure, got {other:?}"),
    }
}
