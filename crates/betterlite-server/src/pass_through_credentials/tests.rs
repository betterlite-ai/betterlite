#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::auth_model_lists::credential::{CredentialEntry, CredentialSnapshot};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{PassThroughDeployment, PassthroughEndpointRouter};

fn resolve_provider(model: &str, custom_provider: Option<&str>) -> Option<String> {
    custom_provider.map(str::to_owned).or_else(|| {
        model
            .split_once('/')
            .map(|(provider, _)| provider.to_owned())
    })
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn obj(pairs: &[(&str, Json)]) -> JsonObject {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn deployment(model_name: &str, params: JsonObject) -> PassThroughDeployment {
    PassThroughDeployment {
        model_name: model_name.to_owned(),
        litellm_params: serde_json::from_value(serde_json::json!(params)).unwrap(),
    }
}

fn available(model_name: &str, upstream: &str, extra: &[(&str, Json)]) -> PassThroughDeployment {
    let mut pairs = vec![
        ("model", text(upstream)),
        ("use_in_pass_through", Json::Bool(true)),
    ];
    pairs.extend(extra.iter().map(|(key, value)| (*key, value.clone())));
    deployment(model_name, obj(&pairs))
}

fn vertex(model_name: &str, project: &str, location: &str) -> PassThroughDeployment {
    let mut candidate = available(model_name, &format!("vertex_ai/{model_name}"), &[]);
    candidate.litellm_params.vertex_project = Some(project.to_owned());
    candidate.litellm_params.vertex_location = Some(location.to_owned());
    candidate
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn snapshot(pairs: &[(&str, JsonObject)]) -> CredentialSnapshot {
    CredentialSnapshot::new(
        pairs
            .iter()
            .map(|(name, values)| CredentialEntry {
                credential_name: (*name).to_owned(),
                credential_values: values.clone(),
            })
            .collect(),
    )
}

fn router(
    deployments: Vec<PassThroughDeployment>,
    env: &[(&str, &str)],
) -> PassthroughEndpointRouter {
    PassthroughEndpointRouter {
        deployments,
        resolve_provider: Arc::new(resolve_provider),
        credentials: CredentialSnapshot::default(),
        environment: Arc::new(environment(env)),
        vertex_by_deployment_key: BTreeMap::new(),
        default_vertex_config: None,
    }
}

mod credential_resolution {
    use super::*;

    fn openai(flag: bool, api_key: &str) -> PassThroughDeployment {
        available(
            "gpt",
            "openai/gpt-4o-mini",
            &[
                ("api_key", text(api_key)),
                ("use_in_pass_through", Json::Bool(flag)),
            ],
        )
    }

    #[test]
    fn a_matching_deployment_key_wins_over_the_environment() {
        let resolver = router(
            vec![openai(true, "deployment-key")],
            &[("OPENAI_API_KEY", "environment-key")],
        );
        let api_key = resolver
            .get_credentials("openai", None)
            .expect("deployment key");
        assert_eq!(api_key.expose_secret(), "deployment-key");
    }

    #[test]
    fn an_unflagged_deployment_leaves_the_environment_to_answer() {
        let resolver = router(
            vec![openai(false, "deployment-key")],
            &[("OPENAI_API_KEY", "environment-key")],
        );
        let api_key = resolver
            .get_credentials("openai", None)
            .expect("environment key");
        assert_eq!(api_key.expose_secret(), "environment-key");
    }

    #[test]
    fn nothing_configured_resolves_to_none() {
        let unflagged = router(vec![openai(false, "deployment-key")], &[]);
        assert!(unflagged.get_credentials("openai", None).is_none());
        let empty = router(vec![], &[]);
        assert!(empty.get_credentials("openai", None).is_none());
        let blank = router(vec![], &[("OPENAI_API_KEY", "")]);
        assert!(blank.get_credentials("openai", None).is_none());
    }
    #[test]
    fn missing_keys_skip_only_matching_deployments() {
        let mut resolver = router(
            vec![
                available("missing", "openai/gpt", &[]),
                available(
                    "unrelated",
                    "anthropic/claude",
                    &[("api_key", text("wrong"))],
                ),
            ],
            &[("OPENAI_API_KEY", "fallback")],
        );
        assert_eq!(
            resolver
                .get_credentials("openai", None)
                .unwrap()
                .expose_secret(),
            "fallback"
        );
        resolver.deployments.push(openai(true, ""));
        assert_eq!(
            resolver
                .get_credentials("openai", None)
                .unwrap()
                .expose_secret(),
            ""
        );
    }
}

mod deployment_matching {
    use super::*;

    fn openai(api_key: &str) -> PassThroughDeployment {
        available("gpt", "openai/gpt-4o-mini", &[("api_key", text(api_key))])
    }

    fn assemblyai(api_base: &str, api_key: &str) -> PassThroughDeployment {
        available(
            "transcribe",
            "assemblyai/universal-2",
            &[("api_base", text(api_base)), ("api_key", text(api_key))],
        )
    }

    #[test]
    fn another_providers_deployment_lends_no_key() {
        let resolver = router(vec![openai("openai-key")], &[]);
        assert!(resolver.get_credentials("anthropic", None).is_none());
        let openai_key = resolver
            .get_credentials("openai", None)
            .expect("openai key");
        assert_eq!(openai_key.expose_secret(), "openai-key");
    }

    #[test]
    fn the_region_must_match_the_deployments_api_base() {
        let resolver = router(
            vec![
                assemblyai("https://api.eu.assemblyai.com", "eu-key"),
                assemblyai("https://api.assemblyai.com", "us-key"),
            ],
            &[],
        );
        let eu = resolver
            .get_credentials("assemblyai", Some("eu"))
            .expect("eu key");
        assert_eq!(eu.expose_secret(), "eu-key");
        let unset = resolver
            .get_credentials("assemblyai", None)
            .expect("us key");
        assert_eq!(unset.expose_secret(), "us-key");
        assert!(
            resolver
                .get_credentials("assemblyai", Some("us-west"))
                .is_none()
        );
    }

    #[test]
    fn only_assemblyai_reads_a_region_from_the_api_base() {
        assert_eq!(
            super::super::region_name_from_api_base(
                "assemblyai",
                Some("https://api.eu.assemblyai.com")
            ),
            Some("eu")
        );
        assert_eq!(
            super::super::region_name_from_api_base(
                "assemblyai",
                Some("https://api.assemblyai.com")
            ),
            None
        );
        assert_eq!(
            super::super::region_name_from_api_base("openai", Some("https://api.eu.openai.com")),
            None
        );
        assert_eq!(
            super::super::region_name_from_api_base("assemblyai", None),
            None
        );
    }

    #[test]
    fn the_first_matching_deployment_wins() {
        let resolver = router(vec![openai("first-key"), openai("second-key")], &[]);
        let api_key = resolver.get_credentials("openai", None).expect("first key");
        assert_eq!(api_key.expose_secret(), "first-key");
    }

    #[rstest]
    #[case::deployment_name("public-gemini", Some("project-one"))]
    #[case::upstream_model("vertex_ai/publishers/google/gemini-2.0", Some("project-one"))]
    #[case::suffix("publishers/google/gemini-2.0", Some("project-one"))]
    #[case::last_component("gemini-2.0", None)]
    fn model_names_match_the_name_and_the_suffix_after_the_first_slash(
        #[case] named: &str,
        #[case] expected: Option<&str>,
    ) {
        let published = available(
            "public-gemini",
            "vertex_ai/publishers/google/gemini-2.0",
            &[
                ("vertex_project", text("project-one")),
                ("vertex_location", text("us-central1")),
            ],
        );
        let other = vertex("other", "project-two", "us-central1");
        let resolver = router(vec![published, other], &[]);
        let project = resolver
            .vertex_deployment_credentials(Some(named))
            .and_then(|record| record.vertex_project);
        assert_eq!(project.as_deref(), expected);
    }

    #[test]
    fn provider_errors_missing_models_and_explicit_providers() {
        let mut resolver = router(
            vec![
                available("unknown", "unknown", &[("api_key", text("wrong"))]),
                deployment("missing", obj(&[("use_in_pass_through", Json::Bool(true))])),
            ],
            &[],
        );
        assert!(resolver.get_credentials("openai", None).is_none());
        resolver.deployments.push(available(
            "explicit",
            "other/model",
            &[
                ("custom_llm_provider", text("openai")),
                ("api_key", text("declared")),
            ],
        ));
        assert_eq!(
            resolver
                .get_credentials("openai", None)
                .unwrap()
                .expose_secret(),
            "declared"
        );
    }
}

mod vertex_credentials {
    use super::*;

    #[test]
    fn a_named_model_returns_its_own_deployment() {
        let resolver = router(
            vec![
                vertex("first", "project-one", "us-central1"),
                vertex("second", "project-two", "us-central1"),
            ],
            &[],
        );
        let named = resolver
            .vertex_deployment_credentials(Some("second"))
            .expect("second project");
        assert_eq!(named.vertex_project.as_deref(), Some("project-two"));
        assert!(resolver.vertex_deployment_credentials(None).is_none());
    }

    #[test]
    fn unanimous_deployments_give_the_single_default() {
        let resolver = router(
            vec![
                vertex("first", "shared-project", "us-central1"),
                vertex("second", "shared-project", "us-central1"),
            ],
            &[],
        );
        let unnamed = resolver
            .vertex_deployment_credentials(None)
            .expect("unanimous default");
        assert_eq!(unnamed.vertex_project.as_deref(), Some("shared-project"));
        assert_eq!(unnamed.vertex_location.as_deref(), Some("us-central1"));
    }

    #[test]
    fn disagreeing_deployments_return_none() {
        let resolver = router(
            vec![
                vertex("first", "project-one", "us-central1"),
                vertex("second", "project-two", "us-central1"),
            ],
            &[],
        );
        assert!(resolver.vertex_deployment_credentials(None).is_none());
    }

    #[test]
    fn a_deployment_without_project_or_location_is_ignored() {
        let incomplete = available(
            "incomplete",
            "vertex_ai/incomplete",
            &[("vertex_project", text("project-one"))],
        );
        let complete = vertex("complete", "project-one", "us-central1");
        let resolver = router(vec![incomplete, complete], &[]);
        let unnamed = resolver
            .vertex_deployment_credentials(None)
            .expect("the complete deployment");
        assert_eq!(unnamed.vertex_project.as_deref(), Some("project-one"));
    }

    #[test]
    fn a_non_vertex_deployment_is_ignored() {
        let resolver = router(vec![available("gpt", "openai/gpt-4o-mini", &[])], &[]);
        assert!(resolver.vertex_deployment_credentials(None).is_none());
    }
    #[rstest]
    #[case::model("model")]
    #[case::project("vertex_project")]
    #[case::location("vertex_location")]
    #[case::enabled("use_in_pass_through")]
    fn incomplete_and_disabled_vertex_deployments_never_answer(#[case] field: &str) {
        let mut params = obj(&[
            ("model", text("vertex_ai/only")),
            ("vertex_project", text("project")),
            ("vertex_location", text("location")),
            ("use_in_pass_through", Json::Bool(true)),
        ]);
        params.shift_remove(field);
        let resolver = router(vec![deployment("only", params)], &[]);
        assert!(
            resolver
                .vertex_deployment_credentials(Some("only"))
                .is_none()
        );
    }

    #[test]
    fn an_unknown_model_uses_only_a_unanimous_identity() {
        let mut resolver = router(vec![vertex("only", "project", "location")], &[]);
        assert_eq!(
            resolver
                .vertex_deployment_credentials(Some("unknown"))
                .unwrap()
                .vertex_project
                .as_deref(),
            Some("project")
        );
        resolver
            .deployments
            .push(vertex("second", "project", "elsewhere"));
        assert!(
            resolver
                .vertex_deployment_credentials(Some("unknown"))
                .is_none()
        );
    }
}

mod vertex_credentials_by_project_location {
    use super::*;

    #[test]
    fn a_stored_project_and_location_return_their_credentials() {
        let mut resolver = router(vec![], &[]);
        resolver.add_vertex_credentials("project-one", "us-central1", Some(text("stored")));
        let stored = resolver
            .get_vertex_credentials(Some("project-one"), Some("us-central1"))
            .expect("stored credentials");
        assert_eq!(stored.vertex_credentials, Some(text("stored")));
        assert!(
            resolver
                .get_vertex_credentials(Some("project-two"), Some("us-central1"))
                .is_none()
        );
    }

    #[test]
    fn a_missing_project_location_or_key_returns_the_default() {
        let mut resolver = router(vec![], &[]);
        resolver.add_vertex_credentials("project-one", "us-central1", Some(text("stored")));
        resolver
            .set_default_vertex_config(Some(obj(&[
                ("vertex_project", text("default-project")),
                ("vertex_location", text("default-location")),
            ])))
            .unwrap();
        let missing_key = resolver
            .get_vertex_credentials(Some("unknown"), Some("us-central1"))
            .expect("default on key miss");
        assert_eq!(
            missing_key.vertex_project.as_deref(),
            Some("default-project")
        );
        let absent = resolver
            .get_vertex_credentials(None, Some("us-central1"))
            .expect("default without a project");
        assert_eq!(absent.vertex_project.as_deref(), Some("default-project"));
        let stored = resolver
            .get_vertex_credentials(Some("project-one"), Some("us-central1"))
            .expect("stored credentials");
        assert_eq!(stored.vertex_project.as_deref(), Some("project-one"));
    }
    #[test]
    fn location_and_empty_keys_and_replacement_are_preserved() {
        let mut resolver = router(vec![], &[]);
        resolver.add_vertex_credentials("", "", Some(text("first")));
        resolver.add_vertex_credentials("", "", Some(text("replacement")));
        let record = resolver.get_vertex_credentials(Some(""), Some("")).unwrap();
        assert_eq!(record.vertex_project.as_deref(), Some(""));
        assert_eq!(record.vertex_location.as_deref(), Some(""));
        assert_eq!(record.vertex_credentials, Some(text("replacement")));
        assert!(resolver.get_vertex_credentials(Some(""), None).is_none());
        assert!(
            resolver
                .get_vertex_credentials(Some(""), Some("elsewhere"))
                .is_none()
        );
    }
}

mod credential_accessor {
    use super::*;

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
}

mod vertex_default_config {
    use super::*;

    #[test]
    fn environment_variables_fill_the_default() {
        let mut resolver = router(
            vec![],
            &[
                ("DEFAULT_VERTEXAI_PROJECT", "environment-project"),
                ("DEFAULT_VERTEXAI_LOCATION", "environment-location"),
                (
                    "DEFAULT_GOOGLE_APPLICATION_CREDENTIALS",
                    "/environment.json",
                ),
            ],
        );
        resolver.set_default_vertex_config(None).unwrap();
        let config = resolver
            .get_vertex_credentials(None, None)
            .expect("environment default");
        assert_eq!(
            config.vertex_project.as_deref(),
            Some("environment-project")
        );
        assert_eq!(
            config.vertex_location.as_deref(),
            Some("environment-location")
        );
        assert_eq!(config.vertex_credentials, Some(text("/environment.json")));
    }

    #[test]
    fn provided_values_resolve_os_environ_references() {
        let mut resolver = router(
            vec![],
            &[
                ("PROJECT", "resolved-project"),
                ("GOOGLE_CREDS", "resolved-credentials"),
            ],
        );
        resolver
            .set_default_vertex_config(Some(obj(&[
                ("vertex_project", text("os.environ/PROJECT")),
                ("vertex_location", text("us-central1")),
                ("vertex_credentials", text("os.environ/GOOGLE_CREDS")),
            ])))
            .unwrap();
        let config = resolver
            .get_vertex_credentials(None, None)
            .expect("provided default");
        assert_eq!(config.vertex_project.as_deref(), Some("resolved-project"));
        assert_eq!(config.vertex_location.as_deref(), Some("us-central1"));
        assert_eq!(
            config.vertex_credentials,
            Some(text("resolved-credentials"))
        );
    }

    #[test]
    fn an_unset_reference_leaves_the_field_absent() {
        let mut resolver = router(vec![], &[("EMPTY", "")]);
        resolver
            .set_default_vertex_config(Some(obj(&[
                ("vertex_project", text("os.environ/MISSING")),
                ("vertex_location", text("os.environ/EMPTY")),
            ])))
            .unwrap();
        let config = resolver
            .get_vertex_credentials(None, None)
            .expect("provided default");
        assert_eq!(config.vertex_project, None);
        assert_eq!(config.vertex_location, None);
    }

    #[test]
    fn empty_environment_variables_count_as_unset() {
        let mut resolver = router(
            vec![],
            &[
                ("DEFAULT_VERTEXAI_PROJECT", ""),
                ("DEFAULT_VERTEXAI_LOCATION", ""),
                ("DEFAULT_GOOGLE_APPLICATION_CREDENTIALS", ""),
            ],
        );
        resolver.set_default_vertex_config(None).unwrap();
        let config = resolver
            .get_vertex_credentials(None, None)
            .expect("empty default");
        assert_eq!(config.vertex_project, None);
        assert_eq!(config.vertex_location, None);
        assert_eq!(config.vertex_credentials, None);
    }

    #[test]
    fn stored_credentials_beat_the_default_for_their_key() {
        let mut resolver = router(vec![], &[]);
        resolver
            .set_default_vertex_config(Some(obj(&[
                ("vertex_project", text("default-project")),
                ("vertex_location", text("default-location")),
            ])))
            .unwrap();
        resolver.add_vertex_credentials("other-project", "us-central1", None);
        let by_key = resolver
            .get_vertex_credentials(Some("other-project"), Some("us-central1"))
            .expect("stored key");
        assert_eq!(by_key.vertex_project.as_deref(), Some("other-project"));
        assert_eq!(by_key.vertex_credentials, None);
        let fallback = resolver
            .get_vertex_credentials(Some("missing-project"), Some("us-central1"))
            .expect("default on key miss");
        assert_eq!(fallback.vertex_project.as_deref(), Some("default-project"));
    }
    #[rstest]
    #[case::project("vertex_project")]
    #[case::location("vertex_location")]
    #[case::credentials("vertex_credentials")]
    #[case::unknown("unknown")]
    fn invalid_config_keeps_the_previous_default(#[case] field: &str) {
        let mut resolver = router(vec![], &[]);
        resolver
            .set_default_vertex_config(Some(obj(&[("vertex_project", text("kept"))])))
            .unwrap();
        let error = resolver
            .set_default_vertex_config(Some(obj(&[(field, Json::Bool(true))])))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("invalid pass-through Vertex config field {field}")
        );
        assert_eq!(
            resolver
                .default_vertex_config
                .as_ref()
                .unwrap()
                .vertex_project
                .as_deref(),
            Some("kept")
        );
    }

    #[test]
    fn explicit_null_empty_and_object_values_do_not_read_defaults() {
        let mut resolver = router(vec![], &[("DEFAULT_VERTEXAI_PROJECT", "ignored")]);
        resolver
            .set_default_vertex_config(Some(obj(&[
                ("vertex_project", Json::Null),
                ("vertex_location", text("")),
                (
                    "vertex_credentials",
                    Json::Object(obj(&[("key", text("value"))])),
                ),
            ])))
            .unwrap();
        let record = resolver.default_vertex_config.as_ref().unwrap();
        assert!(record.vertex_project.is_none());
        assert_eq!(record.vertex_location.as_deref(), Some(""));
        assert_eq!(
            record.vertex_credentials,
            Some(Json::Object(obj(&[("key", text("value"))])))
        );
        resolver.set_default_vertex_config(Some(obj(&[]))).unwrap();
        assert!(
            resolver
                .default_vertex_config
                .as_ref()
                .unwrap()
                .vertex_location
                .is_none()
        );
    }
}

mod credential_identity {
    use super::*;

    fn with_credentials(model_name: &str, credentials: Json) -> PassThroughDeployment {
        available(
            model_name,
            &format!("vertex_ai/{model_name}"),
            &[
                ("vertex_project", text("shared-project")),
                ("vertex_location", text("us-central1")),
                ("vertex_credentials", credentials),
            ],
        )
    }

    #[test]
    fn object_key_order_does_not_split_a_credential() {
        let first = with_credentials(
            "first",
            Json::Object(obj(&[
                ("client_email", text("service@example.test")),
                ("private_key", text("key")),
            ])),
        );
        let second = with_credentials(
            "second",
            Json::Object(obj(&[
                ("private_key", text("key")),
                ("client_email", text("service@example.test")),
            ])),
        );
        let resolver = router(vec![first, second], &[]);
        let unnamed = resolver
            .vertex_deployment_credentials(None)
            .expect("identical credentials");
        assert_eq!(unnamed.vertex_project.as_deref(), Some("shared-project"));
    }

    #[test]
    fn a_different_object_credential_splits_the_deployments() {
        let first = with_credentials("first", Json::Object(obj(&[("private_key", text("one"))])));
        let second = with_credentials("second", Json::Object(obj(&[("private_key", text("two"))])));
        let resolver = router(vec![first, second], &[]);
        assert!(resolver.vertex_deployment_credentials(None).is_none());
    }

    #[test]
    fn a_text_credential_never_matches_an_object_credential() {
        let first = with_credentials("first", text("credentials.json"));
        let second = with_credentials(
            "second",
            Json::Object(obj(&[("private_key", text("credentials.json"))])),
        );
        let resolver = router(vec![first, second], &[]);
        assert!(resolver.vertex_deployment_credentials(None).is_none());
    }

    #[test]
    fn a_text_credential_compares_by_its_own_text() {
        let same = router(
            vec![
                with_credentials("first", text("credentials")),
                with_credentials("second", text("credentials")),
            ],
            &[],
        );
        assert!(same.vertex_deployment_credentials(None).is_some());
    }
    #[test]
    fn serialized_object_text_is_a_distinct_identity() {
        let first = with_credentials("first", text("{\"key\": \"value\"}"));
        let second = with_credentials("second", Json::Object(obj(&[("key", text("value"))])));
        assert!(
            router(vec![first, second], &[])
                .vertex_deployment_credentials(None)
                .is_none()
        );
        let different = router(
            vec![
                with_credentials("one", text("a")),
                with_credentials("two", text("b")),
            ],
            &[],
        );
        assert!(different.vertex_deployment_credentials(None).is_none());
    }
}
