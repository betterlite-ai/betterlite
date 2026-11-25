use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::{JsonObject, ModelCostEntries, ModelRegistry};
use betterlite_models::provider_model_sets::ProviderModelSets;

use super::support::{decimal_text, flag, list, number, object, text};
use crate::model_info_search::enrich::{
    EnrichContext, EnrichError, ModelInfoCatalog, RouterSource, enrich_model_info,
};

struct FakeCatalog {
    entries: Vec<((String, Option<String>), JsonObject)>,
    calls: Mutex<Vec<(String, Option<String>)>>,
}

impl FakeCatalog {
    fn new(entries: Vec<((&str, Option<&str>), JsonObject)>) -> Self {
        let normalized_entries = entries
            .into_iter()
            .map(|((model, provider), value)| {
                ((model.to_owned(), provider.map(str::to_owned)), value)
            })
            .collect();
        Self {
            entries: normalized_entries,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn empty() -> Self {
        Self::new(Vec::new())
    }

    fn calls(&self) -> Vec<(String, Option<String>)> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.clone()
    }
}

#[async_trait]
impl ModelInfoCatalog for FakeCatalog {
    async fn model_info(&self, model: &str, provider: Option<&str>) -> Option<JsonObject> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push((model.to_owned(), provider.map(str::to_owned)));
        drop(calls);
        self.entries
            .iter()
            .find(|((entry_model, entry_provider), _value)| {
                entry_model == model && entry_provider.as_deref() == provider
            })
            .map(|(_key, value)| value.clone())
    }
}

struct FakeRouter {
    client: Option<String>,
    discovered: JsonObject,
}

impl RouterSource for FakeRouter {
    fn async_client_name(&self, _model_id: Option<&str>) -> Option<String> {
        self.client.clone()
    }

    fn discovered_model_info(&self, _model_id: Option<&str>) -> JsonObject {
        self.discovered.clone()
    }
}

fn registry(cost_map: &str) -> ModelRegistry {
    let model_cost: JsonObject = serde_json::from_str(cost_map).expect("a model cost map");
    ModelRegistry {
        model_cost,
        runtime_registered: ModelCostEntries::default(),
        provider_sets: ProviderModelSets::default(),
        keys: ModelCostKeys::default(),
        live_replay: None,
    }
}

fn deployment(backend: &str, info: Vec<(&str, Json)>, params: Vec<(&str, Json)>) -> JsonObject {
    let mut litellm_params = object(vec![("model", text(backend))]);
    for (key, value) in params {
        litellm_params.insert(key.to_owned(), value);
    }
    object(vec![
        ("model_name", text("gpt-4o")),
        ("litellm_params", Json::Object(litellm_params)),
        ("model_info", Json::Object(object(info))),
    ])
}

fn info_of(model: &JsonObject) -> &JsonObject {
    let Some(Json::Object(info)) = model.get("model_info") else {
        panic!("the enriched deployment has a model_info object");
    };
    info
}

fn params_of(model: &JsonObject) -> &JsonObject {
    let Some(Json::Object(params)) = model.get("litellm_params") else {
        panic!("the redacted deployment has a litellm_params object");
    };
    params
}

async fn enrich(
    model: &JsonObject,
    debug: bool,
    registry: &ModelRegistry,
    catalog: &FakeCatalog,
    router: Option<&FakeRouter>,
) -> JsonObject {
    enrich_model_info(
        model,
        debug,
        &EnrichContext {
            registry,
            catalog,
            router: router.map(|source| source as &dyn RouterSource),
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn debug_client_values() {
    let registry = registry("{}");
    let catalog = FakeCatalog::empty();
    let model = deployment("openai/gpt-4o", vec![("id", text("dep-1"))], Vec::new());
    let cold = FakeRouter {
        client: None,
        discovered: JsonObject::new(),
    };
    let no_router = enrich(&model, true, &registry, &catalog, None).await;
    assert_eq!(
        no_router.get("openai_client"),
        Some(&text("llm_router_is_None"))
    );
    let cold_router = enrich(&model, true, &registry, &catalog, Some(&cold)).await;
    assert_eq!(cold_router.get("openai_client"), Some(&text("None")));
}

#[tokio::test]
async fn debug_client_uses_router_client_and_honors_debug_flag() {
    let registry = registry("{}");
    let catalog = FakeCatalog::empty();
    let model = deployment("openai/gpt-4o", vec![("id", text("dep-1"))], Vec::new());
    let warm = FakeRouter {
        client: Some(String::from("an-async-client")),
        discovered: JsonObject::new(),
    };
    let warm_router = enrich(&model, true, &registry, &catalog, Some(&warm)).await;
    assert_eq!(
        warm_router.get("openai_client"),
        Some(&text("an-async-client"))
    );
    let quiet = enrich(&model, false, &registry, &catalog, Some(&warm)).await;
    assert_eq!(quiet.get("openai_client"), None);
}

#[tokio::test]
async fn debug_client_uses_none_for_missing_or_blank_client() {
    let registry = registry("{}");
    let catalog = FakeCatalog::empty();
    let model = deployment("openai/gpt-4o", vec![("id", text("dep-1"))], Vec::new());
    let cold = FakeRouter {
        client: None,
        discovered: JsonObject::new(),
    };
    let anonymous = deployment("openai/gpt-4o", Vec::new(), Vec::new());
    let without_id = enrich(&anonymous, true, &registry, &catalog, Some(&cold)).await;
    assert_eq!(without_id.get("openai_client"), Some(&text("None")));
    let blank = FakeRouter {
        client: Some(String::new()),
        discovered: JsonObject::new(),
    };
    let blank_client = enrich(&model, true, &registry, &catalog, Some(&blank)).await;
    assert_eq!(blank_client.get("openai_client"), Some(&text("None")));
}

#[tokio::test]
async fn catalog_three_pass() {
    assert_catalog_lookup(
        "openai/gpt-4o",
        None,
        "openai/gpt-4o",
        None,
        100,
        vec![("openai/gpt-4o", None)],
    )
    .await;
    assert_catalog_lookup(
        "openai/gpt-4",
        Some(text("gpt-4o")),
        "openai/gpt-4",
        None,
        200,
        vec![("gpt-4o", None), ("openai/gpt-4", None)],
    )
    .await;
    assert_catalog_lookup(
        "openai/gpt-4o",
        None,
        "gpt-4o",
        Some("openai"),
        300,
        vec![
            ("openai/gpt-4o", None),
            ("openai/gpt-4o", None),
            ("gpt-4o", Some("openai")),
        ],
    )
    .await;
}

async fn assert_catalog_lookup(
    backend: &str,
    base_model: Option<Json>,
    catalog_model: &str,
    provider: Option<&str>,
    max_tokens: i64,
    expected_calls: Vec<(&str, Option<&str>)>,
) {
    let registry = registry("{}");
    let mut info_fields = Vec::new();
    if let Some(base) = base_model {
        info_fields.push(("base_model", base));
    }
    let model = deployment(backend, info_fields, Vec::new());
    let catalog = FakeCatalog::new(vec![(
        (catalog_model, provider),
        object(vec![("max_tokens", number(max_tokens))]),
    )]);
    let enriched = enrich(&model, false, &registry, &catalog, None).await;
    assert_eq!(
        info_of(&enriched).get("max_tokens"),
        Some(&number(max_tokens))
    );
    assert_eq!(
        catalog.calls(),
        expected_calls
            .into_iter()
            .map(|(model_name, provider_name)| {
                (String::from(model_name), provider_name.map(String::from))
            })
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn azure_base_model() {
    assert_catalog_lookup(
        "azure/my-deploy",
        Some(text("gpt-4o")),
        "gpt-4o",
        None,
        50,
        vec![("gpt-4o", None)],
    )
    .await;
}

#[tokio::test]
async fn azure_backend_fallback_modes() {
    assert_catalog_lookup(
        "azure/my-deploy2",
        None,
        "azure/my-deploy2",
        None,
        60,
        vec![("azure/my-deploy2", None)],
    )
    .await;
    assert_catalog_lookup(
        "azure/my-deploy3",
        Some(number(5)),
        "azure/my-deploy3",
        None,
        70,
        vec![("azure/my-deploy3", None)],
    )
    .await;
}

#[tokio::test]
async fn discovery_null_precedence() {
    assert_discovery_merge(
        vec![("id", text("dep-1")), ("created_at", Json::Null)],
        vec![("created_at", number(111)), ("max_tokens", number(222))],
        vec![("created_at", number(333)), ("owned_by", text("team"))],
        ("created_at", number(333)),
        ("max_tokens", number(222)),
    )
    .await;
    assert_discovery_merge(
        vec![
            ("id", text("dep-1")),
            ("created_at", number(5)),
            ("region", Json::Null),
        ],
        vec![("region", text("us"))],
        vec![("created_at", number(333))],
        ("created_at", number(5)),
        ("region", Json::Null),
    )
    .await;
}

async fn assert_discovery_merge(
    existing: Vec<(&str, Json)>,
    catalog_fields: Vec<(&str, Json)>,
    discovered_fields: Vec<(&str, Json)>,
    expected_first: (&str, Json),
    expected_second: (&str, Json),
) {
    let registry = registry("{}");
    let catalog = FakeCatalog::new(vec![(("openai/gpt-4o", None), object(catalog_fields))]);
    let router = FakeRouter {
        client: None,
        discovered: object(discovered_fields),
    };
    let model = deployment("openai/gpt-4o", existing, Vec::new());
    let enriched = enrich(&model, false, &registry, &catalog, Some(&router)).await;
    let actual_info = info_of(&enriched);
    assert_eq!(actual_info.get(expected_first.0), Some(&expected_first.1));
    assert_eq!(actual_info.get(expected_second.0), Some(&expected_second.1));
}

#[tokio::test]
async fn pricing_stamps_unpriced() {
    let registry = registry(r#"{"gpt-4o": {"litellm_provider": "openai"}}"#);
    let catalog = FakeCatalog::new(vec![(
        ("openai/gpt-4o", None),
        object(vec![
            ("key", text("gpt-4o")),
            ("input_cost_per_token", decimal_text("0.0001")),
            ("output_cost_per_token", decimal_text("0.0002")),
            ("max_tokens", number(128_000)),
        ]),
    )]);
    let model = deployment(
        "openai/gpt-4o",
        vec![("id", text("dep-1"))],
        vec![("input_cost_per_token", decimal_text("0.5"))],
    );
    let enriched = enrich(&model, false, &registry, &catalog, None).await;
    let stamp_info = info_of(&enriched);
    assert_eq!(
        stamp_info.get("input_cost_per_token"),
        Some(&decimal_text("0.5"))
    );
    assert_eq!(stamp_info.get("output_cost_per_token"), Some(&Json::Null));
    assert_eq!(stamp_info.get("max_tokens"), Some(&number(128_000)));
    assert_eq!(stamp_info.get("model"), None);
    assert_eq!(
        stamp_info.get("pricing_overrides"),
        Some(&list(vec![text("input_cost_per_token")]))
    );
}

#[tokio::test]
async fn credentials_redacted_name_retained() {
    let registry = registry("{}");
    let catalog = FakeCatalog::empty();
    let model = deployment(
        "openai/gpt-4o",
        Vec::new(),
        vec![
            ("api_key", text("sk-1234567890")),
            ("client_secret", text("client-secret-value")),
            ("vertex_credentials", text("{\"type\":\"service_account\"}")),
            (
                "vertex_ai_credentials",
                text("{\"type\":\"external_account\"}"),
            ),
            ("aws_access_key_id", text("AKIA1234567890")),
            ("aws_secret_access_key", text("aws-secret-value")),
            ("litellm_credential_name", text("cred-1")),
            ("database_password", text("hunter2secret")),
            ("default_api_key_tpm_limit", number(10)),
            ("default_api_key_rpm_limit", number(20)),
        ],
    );
    let enriched = enrich(&model, false, &registry, &catalog, None).await;
    let params = params_of(&enriched);
    assert_eq!(params.get("api_key"), None);
    assert_eq!(params.get("client_secret"), None);
    assert_eq!(params.get("vertex_credentials"), None);
    assert_eq!(params.get("vertex_ai_credentials"), None);
    assert_eq!(params.get("aws_access_key_id"), None);
    assert_eq!(params.get("aws_secret_access_key"), None);
    assert_eq!(params.get("litellm_credential_name"), Some(&text("cred-1")));
    assert_eq!(params.get("default_api_key_tpm_limit"), Some(&number(10)));
    assert_eq!(params.get("default_api_key_rpm_limit"), Some(&number(20)));
    assert!(matches!(
        params.get("database_password"),
        Some(Json::Text(value)) if value != "hunter2secret"
    ));
}

#[tokio::test]
async fn malformed_deployment_is_an_error() {
    let missing_params = object(vec![
        ("model_name", text("gpt-4o")),
        ("model_info", Json::Object(object(Vec::new()))),
    ]);
    expect_enrich_error(missing_params, ErrorKind::Redact).await;
    let malformed_info = object(vec![
        ("model_name", text("gpt-4o")),
        (
            "litellm_params",
            Json::Object(object(vec![("model", text("openai/gpt-4o"))])),
        ),
        ("model_info", text("not an object")),
    ]);
    expect_enrich_error(malformed_info, ErrorKind::ModelInfo).await;
    let bad_backend = object(vec![
        ("model_name", text("gpt-4o")),
        (
            "litellm_params",
            Json::Object(object(vec![("model", flag(true))])),
        ),
        ("model_info", Json::Object(object(Vec::new()))),
    ]);
    expect_enrich_error(bad_backend, ErrorKind::Backend).await;
}

enum ErrorKind {
    Redact,
    ModelInfo,
    Backend,
}

async fn expect_enrich_error(model: JsonObject, kind: ErrorKind) {
    let registry = registry("{}");
    let catalog = FakeCatalog::empty();
    let context = EnrichContext {
        registry: &registry,
        catalog: &catalog,
        router: None,
    };
    let error = enrich_model_info(&model, false, &context)
        .await
        .unwrap_err();
    match kind {
        ErrorKind::Redact => assert!(matches!(error, EnrichError::Redact(_))),
        ErrorKind::ModelInfo => assert!(matches!(error, EnrichError::ModelInfoNotObject)),
        ErrorKind::Backend => assert!(matches!(error, EnrichError::ModelNotText)),
    }
}

#[tokio::test]
async fn a_priced_cost_map_keeps_the_catalog_prices() {
    let registry = registry(r#"{"gpt-4o": {"input_cost_per_token": 0.00001}}"#);
    let catalog = FakeCatalog::new(vec![(
        ("openai/gpt-4o", None),
        object(vec![
            ("key", text("gpt-4o")),
            ("input_cost_per_token", decimal_text("0.0001")),
            ("output_cost_per_token", decimal_text("0.0002")),
        ]),
    )]);
    let model = deployment("openai/gpt-4o", vec![("id", text("dep-1"))], Vec::new());
    let enriched = enrich_model_info(
        &model,
        false,
        &EnrichContext {
            registry: &registry,
            catalog: &catalog,
            router: None,
        },
    )
    .await
    .unwrap();
    let info = info_of(&enriched);
    assert_eq!(
        info.get("input_cost_per_token"),
        Some(&decimal_text("0.0001"))
    );
    assert_eq!(
        info.get("output_cost_per_token"),
        Some(&decimal_text("0.0002"))
    );
}
