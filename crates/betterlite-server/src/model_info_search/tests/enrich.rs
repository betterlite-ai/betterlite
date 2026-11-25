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
        let entries = entries
            .into_iter()
            .map(|((model, provider), value)| {
                ((model.to_owned(), provider.map(str::to_owned)), value)
            })
            .collect();
        Self {
            entries,
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

#[tokio::test]
async fn debug_client_values() {
    let registry = registry("{}");
    let catalog = FakeCatalog::empty();
    let model = deployment("openai/gpt-4o", vec![("id", text("dep-1"))], Vec::new());
    let cold = FakeRouter {
        client: None,
        discovered: JsonObject::new(),
    };
    let no_router = enrich_model_info(
        &model,
        true,
        &EnrichContext {
            registry: &registry,
            catalog: &catalog,
            router: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        no_router.get("openai_client"),
        Some(&text("llm_router_is_None"))
    );
    let cold_router = enrich_model_info(
        &model,
        true,
        &EnrichContext {
            registry: &registry,
            catalog: &catalog,
            router: Some(&cold),
        },
    )
    .await
    .unwrap();
    assert_eq!(cold_router.get("openai_client"), Some(&text("None")));
    let warm = FakeRouter {
        client: Some(String::from("an-async-client")),
        discovered: JsonObject::new(),
    };
    let warm_router = enrich_model_info(
        &model,
        true,
        &EnrichContext {
            registry: &registry,
            catalog: &catalog,
            router: Some(&warm),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        warm_router.get("openai_client"),
        Some(&text("an-async-client"))
    );
    let quiet = enrich_model_info(
        &model,
        false,
        &EnrichContext {
            registry: &registry,
            catalog: &catalog,
            router: Some(&warm),
        },
    )
    .await
    .unwrap();
    assert_eq!(quiet.get("openai_client"), None);
    let anonymous = deployment("openai/gpt-4o", Vec::new(), Vec::new());
    let without_id = enrich_model_info(
        &anonymous,
        true,
        &EnrichContext {
            registry: &registry,
            catalog: &catalog,
            router: Some(&cold),
        },
    )
    .await
    .unwrap();
    assert_eq!(without_id.get("openai_client"), Some(&text("None")));
    let blank = FakeRouter {
        client: Some(String::new()),
        discovered: JsonObject::new(),
    };
    let blank_client = enrich_model_info(
        &model,
        true,
        &EnrichContext {
            registry: &registry,
            catalog: &catalog,
            router: Some(&blank),
        },
    )
    .await
    .unwrap();
    assert_eq!(blank_client.get("openai_client"), Some(&text("None")));
}

#[tokio::test]
async fn catalog_three_pass() {
    let registry = registry("{}");
    let model = deployment("openai/gpt-4o", Vec::new(), Vec::new());
    let first = FakeCatalog::new(vec![(
        ("openai/gpt-4o", None),
        object(vec![("max_tokens", number(100))]),
    )]);
    let enriched = enrich_model_info(
        &model,
        false,
        &EnrichContext {
            registry: &registry,
            catalog: &first,
            router: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(info_of(&enriched).get("max_tokens"), Some(&number(100)));
    assert_eq!(first.calls(), vec![(String::from("openai/gpt-4o"), None)]);

    let based = deployment(
        "openai/gpt-4",
        vec![("base_model", text("gpt-4o"))],
        Vec::new(),
    );
    let second = FakeCatalog::new(vec![(
        ("openai/gpt-4", None),
        object(vec![("max_tokens", number(200))]),
    )]);
    let enriched = enrich_model_info(
        &based,
        false,
        &EnrichContext {
            registry: &registry,
            catalog: &second,
            router: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(info_of(&enriched).get("max_tokens"), Some(&number(200)));
    assert_eq!(
        second.calls(),
        vec![
            (String::from("gpt-4o"), None),
            (String::from("openai/gpt-4"), None),
        ]
    );

    let third = FakeCatalog::new(vec![(
        ("gpt-4o", Some("openai")),
        object(vec![("max_tokens", number(300))]),
    )]);
    let enriched = enrich_model_info(
        &model,
        false,
        &EnrichContext {
            registry: &registry,
            catalog: &third,
            router: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(info_of(&enriched).get("max_tokens"), Some(&number(300)));
    assert_eq!(
        third.calls(),
        vec![
            (String::from("openai/gpt-4o"), None),
            (String::from("openai/gpt-4o"), None),
            (String::from("gpt-4o"), Some(String::from("openai"))),
        ]
    );
}

#[tokio::test]
async fn azure_base_model() {
    let registry = registry("{}");
    let base = deployment(
        "azure/my-deploy",
        vec![("base_model", text("gpt-4o"))],
        Vec::new(),
    );
    let catalog = FakeCatalog::new(vec![(
        ("gpt-4o", None),
        object(vec![("max_tokens", number(50))]),
    )]);
    let enriched = enrich_model_info(
        &base,
        false,
        &EnrichContext {
            registry: &registry,
            catalog: &catalog,
            router: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(info_of(&enriched).get("max_tokens"), Some(&number(50)));
    assert_eq!(catalog.calls(), vec![(String::from("gpt-4o"), None)]);

    let plain_azure = deployment("azure/my-deploy2", Vec::new(), Vec::new());
    let backend_catalog = FakeCatalog::new(vec![(
        ("azure/my-deploy2", None),
        object(vec![("max_tokens", number(60))]),
    )]);
    let enriched = enrich_model_info(
        &plain_azure,
        false,
        &EnrichContext {
            registry: &registry,
            catalog: &backend_catalog,
            router: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(info_of(&enriched).get("max_tokens"), Some(&number(60)));
    assert_eq!(
        backend_catalog.calls(),
        vec![(String::from("azure/my-deploy2"), None)]
    );

    let odd_base = deployment(
        "azure/my-deploy3",
        vec![("base_model", number(5))],
        Vec::new(),
    );
    let unnamed = FakeCatalog::new(vec![(
        ("azure/my-deploy3", None),
        object(vec![("max_tokens", number(70))]),
    )]);
    let enriched = enrich_model_info(
        &odd_base,
        false,
        &EnrichContext {
            registry: &registry,
            catalog: &unnamed,
            router: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(info_of(&enriched).get("max_tokens"), Some(&number(70)));
    assert_eq!(
        unnamed.calls(),
        vec![(String::from("azure/my-deploy3"), None)]
    );
}

#[tokio::test]
async fn discovery_null_precedence() {
    let registry = registry("{}");
    let catalog = FakeCatalog::new(vec![(
        ("openai/gpt-4o", None),
        object(vec![
            ("created_at", number(111)),
            ("max_tokens", number(222)),
        ]),
    )]);
    let router = FakeRouter {
        client: None,
        discovered: object(vec![
            ("created_at", number(333)),
            ("owned_by", text("team")),
        ]),
    };
    let model = deployment(
        "openai/gpt-4o",
        vec![("id", text("dep-1")), ("created_at", Json::Null)],
        Vec::new(),
    );
    let enriched = enrich_model_info(
        &model,
        false,
        &EnrichContext {
            registry: &registry,
            catalog: &catalog,
            router: Some(&router),
        },
    )
    .await
    .unwrap();
    let info = info_of(&enriched);
    assert_eq!(info.get("created_at"), Some(&number(333)));
    assert_eq!(info.get("max_tokens"), Some(&number(222)));
    assert_eq!(info.get("owned_by"), Some(&text("team")));

    let fixed_catalog = FakeCatalog::new(vec![(
        ("openai/gpt-4o", None),
        object(vec![("region", text("us"))]),
    )]);
    let fixed_router = FakeRouter {
        client: None,
        discovered: object(vec![("created_at", number(333))]),
    };
    let fixed = deployment(
        "openai/gpt-4o",
        vec![
            ("id", text("dep-1")),
            ("created_at", number(5)),
            ("region", Json::Null),
        ],
        Vec::new(),
    );
    let enriched = enrich_model_info(
        &fixed,
        false,
        &EnrichContext {
            registry: &registry,
            catalog: &fixed_catalog,
            router: Some(&fixed_router),
        },
    )
    .await
    .unwrap();
    let info = info_of(&enriched);
    assert_eq!(info.get("created_at"), Some(&number(5)));
    assert_eq!(info.get("region"), Some(&Json::Null));
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
    assert_eq!(info.get("input_cost_per_token"), Some(&decimal_text("0.5")));
    assert_eq!(info.get("output_cost_per_token"), Some(&Json::Null));
    assert_eq!(info.get("max_tokens"), Some(&number(128_000)));
    assert_eq!(
        info.get("pricing_overrides"),
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
    let registry = registry("{}");
    let catalog = FakeCatalog::empty();
    let context = EnrichContext {
        registry: &registry,
        catalog: &catalog,
        router: None,
    };
    let no_params = object(vec![
        ("model_name", text("gpt-4o")),
        ("model_info", Json::Object(object(Vec::new()))),
    ]);
    let error = enrich_model_info(&no_params, false, &context)
        .await
        .unwrap_err();
    assert!(matches!(error, EnrichError::Redact(_)));
    let bad_info = object(vec![
        ("model_name", text("gpt-4o")),
        (
            "litellm_params",
            Json::Object(object(vec![("model", text("openai/gpt-4o"))])),
        ),
        ("model_info", text("not an object")),
    ]);
    let error = enrich_model_info(&bad_info, false, &context)
        .await
        .unwrap_err();
    assert!(matches!(error, EnrichError::ModelInfoNotObject));
    let bad_backend = object(vec![
        ("model_name", text("gpt-4o")),
        (
            "litellm_params",
            Json::Object(object(vec![("model", flag(true))])),
        ),
        ("model_info", Json::Object(object(Vec::new()))),
    ]);
    let error = enrich_model_info(&bad_backend, false, &context)
        .await
        .unwrap_err();
    assert!(matches!(error, EnrichError::ModelNotText));
}
