#![cfg(test)]

use secrecy::ExposeSecret as _;

use super::{
    Arc, BTreeMap, Json, JsonObject, PassthroughEndpointRouter, available, environment, obj,
    resolve_provider, router, snapshot, text, vertex,
};

fn resolver_with(credentials: JsonObject) -> PassthroughEndpointRouter {
    PassthroughEndpointRouter {
        deployments: vec![available(
            "gpt",
            "openai/gpt-4o-mini",
            &[
                ("litellm_credential_name", text("named")),
                ("api_key", text("parameter-key")),
            ],
        )],
        resolve_provider: Arc::new(resolve_provider),
        credentials: snapshot(&[("named", credentials)]),
        environment: Arc::new(environment(&[])),
        vertex_by_deployment_key: BTreeMap::new(),
        default_vertex_config: None,
    }
}

#[test]
fn stored_values_override_litellm_params() {
    let resolver = resolver_with(obj(&[("api_key", text("stored-key"))]));
    let api_key = resolver
        .get_credentials("openai", None)
        .expect("stored key");
    assert_eq!(api_key.expose_secret(), "stored-key");
}

#[test]
fn an_absent_stored_value_falls_back_to_litellm_params() {
    let resolver = resolver_with(obj(&[]));
    let api_key = resolver
        .get_credentials("openai", None)
        .expect("parameter key");
    assert_eq!(api_key.expose_secret(), "parameter-key");
}

#[test]
fn a_non_text_stored_value_falls_back_to_litellm_params() {
    let resolver = resolver_with(obj(&[("api_key", Json::Bool(true))]));
    let api_key = resolver
        .get_credentials("openai", None)
        .expect("parameter key");
    assert_eq!(api_key.expose_secret(), "parameter-key");
}

#[test]
fn stored_vertex_credentials_override_litellm_params() {
    let resolver = PassthroughEndpointRouter {
        deployments: vec![available(
            "gemini",
            "vertex_ai/gemini",
            &[
                ("vertex_project", text("project")),
                ("vertex_location", text("us-central1")),
                ("vertex_credentials", text("parameter-credentials")),
                ("litellm_credential_name", text("named")),
            ],
        )],
        resolve_provider: Arc::new(resolve_provider),
        credentials: snapshot(&[(
            "named",
            obj(&[(
                "vertex_credentials",
                Json::Object(obj(&[("key", text("stored"))])),
            )]),
        )]),
        environment: Arc::new(environment(&[])),
        vertex_by_deployment_key: BTreeMap::new(),
        default_vertex_config: None,
    };
    let credentials = resolver
        .vertex_deployment_credentials(Some("gemini"))
        .expect("deployment credentials");
    assert_eq!(
        credentials.vertex_credentials,
        Some(Json::Object(obj(&[("key", text("stored"))])))
    );
}
#[test]
fn stored_base_controls_the_matching_region() {
    let mut resolver = resolver_with(obj(&[("api_base", text("eu"))]));
    resolver.deployments = vec![available(
        "speech",
        "assemblyai/speech",
        &[
            ("litellm_credential_name", text("named")),
            ("api_base", text("us")),
            ("api_key", text("key")),
        ],
    )];
    assert!(resolver.get_credentials("assemblyai", None).is_none());
    assert_eq!(
        resolver
            .get_credentials("assemblyai", Some("eu"))
            .unwrap()
            .expose_secret(),
        "key"
    );
}

#[test]
fn stored_vertex_fields_override_or_fall_back_by_type() {
    let mut resolver = router(vec![vertex("gemini", "project", "location")], &[]);
    let params = &mut resolver.deployments.first_mut().unwrap().litellm_params;
    params.litellm_credential_name = Some("named".to_owned());
    params.vertex_credentials = Some(text("parameter"));
    resolver.credentials = snapshot(&[(
        "named",
        obj(&[
            ("vertex_project", text("stored-project")),
            ("vertex_location", text("stored-location")),
            ("vertex_credentials", text("stored")),
        ]),
    )]);
    let stored = resolver.vertex_deployment_credentials(None).unwrap();
    assert_eq!(stored.vertex_project.as_deref(), Some("stored-project"));
    assert_eq!(stored.vertex_location.as_deref(), Some("stored-location"));
    assert_eq!(stored.vertex_credentials, Some(text("stored")));
    resolver.credentials = snapshot(&[(
        "named",
        obj(&[
            ("vertex_project", Json::Bool(true)),
            ("vertex_location", Json::Null),
            ("vertex_credentials", Json::List(vec![])),
        ]),
    )]);
    let fallback = resolver.vertex_deployment_credentials(None).unwrap();
    assert_eq!(fallback.vertex_project.as_deref(), Some("project"));
    assert_eq!(fallback.vertex_location.as_deref(), Some("location"));
    assert_eq!(fallback.vertex_credentials, Some(text("parameter")));
}
