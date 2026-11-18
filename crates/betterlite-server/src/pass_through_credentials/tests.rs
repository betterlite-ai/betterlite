#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::{
    PassThroughCredentialSource, PassThroughDeployment, PassthroughEndpointRouter,
    VertexPassThroughCredentials,
};

#[derive(Default)]
struct Source {
    deployments: Vec<PassThroughDeployment>,
    credentials: BTreeMap<String, JsonObject>,
}
impl PassThroughCredentialSource for Source {
    fn deployments(&self) -> Vec<PassThroughDeployment> {
        self.deployments.clone()
    }
    fn provider_for(&self, model: &str, provider: Option<&str>) -> Option<String> {
        provider
            .map(str::to_owned)
            .or_else(|| model.split_once('/').map(|(prefix, _)| prefix.to_owned()))
    }
    fn credential_values(&self, name: &str) -> Option<JsonObject> {
        self.credentials.get(name).cloned()
    }
}
fn obj(pairs: &[(&str, Json)]) -> JsonObject {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}
fn deployment(name: Option<&str>, params: JsonObject) -> PassThroughDeployment {
    PassThroughDeployment {
        model_name: name.map(str::to_owned),
        litellm_params: params,
    }
}
fn router(source: Source, env: &[(&str, &str)]) -> PassthroughEndpointRouter {
    let environment = env
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    PassthroughEndpointRouter::new(Some(Arc::new(source)), Arc::new(environment))
}

#[test]
fn credential_resolution_deployment_then_environment() {
    let params = obj(&[
        ("model", Json::Text(String::from("openai/gpt"))),
        ("use_in_pass_through", Json::Bool(true)),
        ("api_key", Json::Text(String::from("deployment"))),
    ]);
    let resolver = router(
        Source {
            deployments: vec![deployment(None, params)],
            credentials: BTreeMap::new(),
        },
        &[("OPENAI_API_KEY", "environment")],
    );
    assert_eq!(
        resolver.get_credentials("openai", None).as_deref(),
        Some("deployment")
    );
    let empty = router(Source::default(), &[("OPENAI_API_KEY", "environment")]);
    assert_eq!(
        empty.get_credentials("openai", None).as_deref(),
        Some("environment")
    );
}

#[test]
fn deployment_matching_provider_region_alias_and_order() {
    let unrelated = deployment(
        None,
        obj(&[
            ("model", Json::Text(String::from("openai/gpt"))),
            ("use_in_pass_through", Json::Bool(true)),
            ("api_key", Json::Text(String::from("wrong"))),
        ]),
    );
    let match_deployment = deployment(
        None,
        obj(&[
            ("model", Json::Text(String::from("assemblyai/transcribe"))),
            ("use_in_pass_through", Json::Bool(true)),
            (
                "api_base",
                Json::Text(String::from("https://api.eu.assemblyai.com")),
            ),
            ("api_key", Json::Text(String::from("right"))),
        ]),
    );
    let resolver = router(
        Source {
            deployments: vec![unrelated, match_deployment],
            credentials: BTreeMap::new(),
        },
        &[],
    );
    assert_eq!(
        resolver
            .get_credentials("assemblyai", Some("eu"))
            .as_deref(),
        Some("right")
    );
    assert_eq!(resolver.get_credentials("assemblyai", None), None);
    assert_eq!(
        PassthroughEndpointRouter::_get_region_name_from_api_base(
            "assemblyai",
            Some("https://foo-eu.test")
        ),
        Some("eu")
    );
    let alias = deployment(
        None,
        obj(&[
            ("model", Json::Text(String::from("vertex_ai/model-name"))),
            ("use_in_pass_through", Json::Bool(true)),
            ("vertex_project", Json::Text(String::from("p"))),
            ("vertex_location", Json::Text(String::from("us"))),
        ]),
    );
    let alias_router = router(
        Source {
            deployments: vec![alias],
            credentials: BTreeMap::new(),
        },
        &[],
    );
    assert_eq!(
        alias_router
            .vertex_deployment_credentials(Some("model-name"))
            .unwrap()
            .vertex_project
            .as_deref(),
        Some("p")
    );
    let nested_alias = deployment(
        None,
        obj(&[
            (
                "model",
                Json::Text(String::from("vertex_ai/publishers/google/model-name")),
            ),
            ("use_in_pass_through", Json::Bool(true)),
            ("vertex_project", Json::Text(String::from("p"))),
            ("vertex_location", Json::Text(String::from("us"))),
        ]),
    );
    let other = deployment(
        None,
        obj(&[
            ("model", Json::Text(String::from("vertex_ai/other"))),
            ("use_in_pass_through", Json::Bool(true)),
            ("vertex_project", Json::Text(String::from("q"))),
            ("vertex_location", Json::Text(String::from("us"))),
        ]),
    );
    let nested_router = router(
        Source {
            deployments: vec![nested_alias, other],
            credentials: BTreeMap::new(),
        },
        &[],
    );
    assert_eq!(
        nested_router.vertex_deployment_credentials(Some("model-name")),
        None
    );
}

#[test]
fn vertex_credentials_named_and_unanimous() {
    let params = obj(&[
        ("model", Json::Text(String::from("vertex_ai/gemini"))),
        ("use_in_pass_through", Json::Bool(true)),
        ("vertex_project", Json::Text(String::from("p"))),
        ("vertex_location", Json::Text(String::from("us"))),
    ]);
    let resolver = router(
        Source {
            deployments: vec![deployment(Some("gemini"), params)],
            credentials: BTreeMap::new(),
        },
        &[],
    );
    let named = resolver
        .vertex_deployment_credentials(Some("gemini"))
        .unwrap();
    assert_eq!(named.vertex_project.as_deref(), Some("p"));
    assert_eq!(resolver.vertex_deployment_credentials(None), Some(named));
}

#[test]
fn vertex_credentials_by_project_location() {
    let mut resolver = router(Source::default(), &[]);
    resolver.add_vertex_credentials("project", "region", Some(Json::Text(String::from("creds"))));
    assert_eq!(
        resolver
            .get_vertex_credentials(Some("project"), Some("region"))
            .unwrap()
            .vertex_credentials,
        Some(Json::Text(String::from("creds")))
    );
    assert_eq!(resolver.get_vertex_credentials(None, Some("region")), None);
}

#[test]
fn credential_accessor_override() {
    let mut source = Source::default();
    source.credentials.insert(
        String::from("creds"),
        obj(&[("api_key", Json::Text(String::from("from-accessor")))]),
    );
    let params = obj(&[
        ("model", Json::Text(String::from("openai/gpt"))),
        ("use_in_pass_through", Json::Bool(true)),
        ("litellm_credential_name", Json::Text(String::from("creds"))),
        ("api_key", Json::Text(String::from("from-param"))),
    ]);
    let resolver = router(
        Source {
            deployments: vec![deployment(None, params)],
            ..source
        },
        &[],
    );
    assert_eq!(
        resolver.get_credentials("openai", None).as_deref(),
        Some("from-accessor")
    );

    let mut source = Source::default();
    source.credentials.insert(
        String::from("creds"),
        obj(&[("api_key", Json::Number(serde_json::Number::from(7)))]),
    );
    let params = obj(&[
        ("model", Json::Text(String::from("openai/gpt"))),
        ("use_in_pass_through", Json::Bool(true)),
        ("litellm_credential_name", Json::Text(String::from("creds"))),
        ("api_key", Json::Text(String::from("param-fallback"))),
    ]);
    let resolver = router(
        Source {
            deployments: vec![deployment(None, params)],
            ..source
        },
        &[],
    );
    assert_eq!(
        resolver.get_credentials("openai", None).as_deref(),
        Some("param-fallback")
    );
}

#[test]
fn vertex_default_config_environment_references() {
    let mut resolver = router(
        Source::default(),
        &[
            ("PROJECT", "p"),
            ("GOOGLE_CREDS", "credentials-json"),
            ("DEFAULT_VERTEXAI_LOCATION", "us"),
        ],
    );
    resolver.set_default_vertex_config(Some(obj(&[
        (
            "vertex_project",
            Json::Text(String::from("os.environ/PROJECT")),
        ),
        ("vertex_location", Json::Text(String::from("us"))),
        (
            "vertex_credentials",
            Json::Text(String::from("os.environ/GOOGLE_CREDS")),
        ),
    ])));
    assert_eq!(
        resolver
            .get_vertex_credentials(None, None)
            .unwrap()
            .vertex_project
            .as_deref(),
        Some("p")
    );
    assert_eq!(
        resolver
            .get_vertex_credentials(None, None)
            .unwrap()
            .vertex_credentials,
        Some(Json::Text(String::from("credentials-json")))
    );
    resolver.set_default_vertex_config(None);
    assert_eq!(
        resolver
            .get_vertex_credentials(None, None)
            .unwrap()
            .vertex_location
            .as_deref(),
        Some("us")
    );
}

#[test]
fn credential_identity_sorted_objects() {
    let first = obj(&[
        ("a", Json::Text(String::from("1"))),
        ("b", Json::Text(String::from("2"))),
    ]);
    let second = obj(&[
        ("b", Json::Text(String::from("2"))),
        ("a", Json::Text(String::from("1"))),
    ]);
    assert_eq!(
        super::credential_identity(Some(&Json::Object(first))),
        super::credential_identity(Some(&Json::Object(second)))
    );
    assert_ne!(
        super::credential_identity(Some(&Json::Text(String::from("x")))),
        super::credential_identity(Some(&Json::Object(JsonObject::new())))
    );
    assert_eq!(
        super::credential_identity(Some(&Json::Text(String::from("credential")))),
        Some(String::from("credential"))
    );
    assert_eq!(
        super::credential_identity(Some(&Json::Object(obj(&[
            ("a", Json::Text(String::from("1"))),
            ("b", Json::Text(String::from("2"))),
        ])))),
        Some(r#"{"a":"1","b":"2"}"#.into())
    );
}

#[test]
fn vertex_disagreement_returns_none() {
    let one = deployment(
        None,
        obj(&[
            ("model", Json::Text(String::from("vertex_ai/a"))),
            ("use_in_pass_through", Json::Bool(true)),
            ("vertex_project", Json::Text(String::from("a"))),
            ("vertex_location", Json::Text(String::from("us"))),
        ]),
    );
    let two = deployment(
        None,
        obj(&[
            ("model", Json::Text(String::from("vertex_ai/b"))),
            ("use_in_pass_through", Json::Bool(true)),
            ("vertex_project", Json::Text(String::from("b"))),
            ("vertex_location", Json::Text(String::from("us"))),
        ]),
    );
    let resolver = router(
        Source {
            deployments: vec![one, two],
            credentials: BTreeMap::new(),
        },
        &[],
    );
    assert_eq!(resolver.vertex_deployment_credentials(None), None);
    assert_eq!(
        resolver
            .vertex_deployment_credentials(Some("vertex_ai/b"))
            .unwrap()
            .vertex_project
            .as_deref(),
        Some("b")
    );
}

#[test]
fn default_vertex_credentials_record_is_nullable() {
    assert_eq!(
        VertexPassThroughCredentials::default().vertex_credentials,
        None
    );
}

mod credential_resolution {
    #[test]
    fn deployment_then_environment() {
        super::credential_resolution_deployment_then_environment();
    }
}
mod deployment_matching {
    #[test]
    fn provider_region_alias_and_order() {
        super::deployment_matching_provider_region_alias_and_order();
    }
}
mod vertex_credentials {
    #[test]
    fn named_and_unanimous() {
        super::vertex_credentials_named_and_unanimous();
    }
    #[test]
    fn disagreement_returns_none() {
        super::vertex_disagreement_returns_none();
    }
}
mod vertex_credentials_by_project_location {
    #[test]
    fn key_hit_and_miss() {
        super::vertex_credentials_by_project_location();
    }
}
mod credential_accessor {
    #[test]
    fn stored_value_overrides_param() {
        super::credential_accessor_override();
    }
}
mod vertex_default_config {
    #[test]
    fn environment_references_and_environment_defaults() {
        super::vertex_default_config_environment_references();
    }
}
mod credential_identity {
    #[test]
    fn object_key_order_and_type() {
        super::credential_identity_sorted_objects();
    }
}
