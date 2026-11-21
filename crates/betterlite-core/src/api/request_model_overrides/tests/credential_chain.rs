use betterlite_auth::auth_model_lists::credential::CredentialEntry;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::support::{
    TestCatalog, alias_map, build_service, catalog, credential, credential_settings, deployment,
    enforced_settings, input, model_config, model_config_field, request, text_field,
};
use crate::api::request_destination::RequestData;
use crate::api::request_model_overrides::catalog::DeploymentParams;
use crate::api::request_model_overrides::diagnostics::OverrideDiagnostic;
use crate::api::request_model_overrides::error::RequestModelOverridesError;
use crate::api::request_model_overrides::{
    OverridesPrincipal, RequestModelOverridesInput, RequestModelOverridesService,
};

fn principal_with(config: &[(&str, &str, Option<&str>)]) -> OverridesPrincipal {
    OverridesPrincipal {
        team_id: None,
        team_model_aliases: alias_map(&[]),
        aliases: alias_map(&[]),
        metadata: None,
        team_metadata: Some(model_config(config)),
        project_metadata: None,
    }
}

fn team_credentials() -> Vec<CredentialEntry> {
    vec![
        credential("openai-cred", &[("api_key", "sk-openai")]),
        credential("azure-cred", &[("api_key", "sk-azure")]),
        credential("team-azure", &[("api_key", "sk-team")]),
    ]
}

#[rstest]
#[case::client_api_key_present(
    "api_key",
    Json::Text("client-key".to_owned()),
    Some("client-key"),
    Vec::new()
)]
#[case::client_api_base_present(
    "api_base",
    Json::Text("https://client.example".to_owned()),
    None,
    Vec::new()
)]
#[case::empty_api_key_counts_as_present("api_key", Json::Text(String::new()), Some(""), Vec::new())]
#[case::null_api_key_still_runs_the_walk(
    "api_key",
    Json::Null,
    None,
    vec![OverrideDiagnostic::CredentialApplied {
        credential: "team-azure".to_owned(),
        model: "gpt-4o".to_owned(),
    }]
)]
#[case::null_api_base_still_runs_the_walk(
    "api_base",
    Json::Null,
    Some("sk-team"),
    vec![OverrideDiagnostic::CredentialApplied {
        credential: "team-azure".to_owned(),
        model: "gpt-4o".to_owned(),
    }]
)]
fn client_credential_fields_gate_the_walk(
    #[case] field: &str,
    #[case] value: Json,
    #[case] expected_key: Option<&str>,
    #[case] expected_diagnostics: Vec<OverrideDiagnostic>,
) {
    let mut data = request(&[("model", "gpt-4o")]);
    data.insert(field.to_owned(), value);
    let principal = principal_with(&[("defaultconfig", "azure", Some("team-azure"))]);
    let input = RequestModelOverridesInput {
        data,
        principal,
        premium_user: true,
    };
    let service = build_service(credential_settings(), None, team_credentials());
    let result = service.apply(&input).unwrap();
    assert_eq!(text_field(&result.data, "api_key"), expected_key);
    assert_eq!(result.diagnostics, expected_diagnostics);
}

#[test]
fn provider_prefix_selects_the_matching_entry() {
    let principal = principal_with(&[
        ("azure/gpt-4", "openai", Some("openai-cred")),
        ("azure/gpt-4", "azure", Some("azure-cred")),
    ]);
    let service = build_service(credential_settings(), None, team_credentials());
    let result = service.apply(&input(principal, "azure/gpt-4")).unwrap();
    assert_eq!(text_field(&result.data, "api_key"), Some("sk-azure"));
}

#[test]
fn deployment_provider_selects_the_matching_entry() {
    let catalog_view =
        catalog(TestCatalog::default().with_group("gpt-4", deployment("azure", "openai/gpt-4")));
    let principal = principal_with(&[
        ("gpt-4", "openai", Some("openai-cred")),
        ("gpt-4", "azure", Some("azure-cred")),
    ]);
    let service = build_service(credential_settings(), catalog_view, team_credentials());
    let result = service.apply(&input(principal, "gpt-4")).unwrap();
    assert_eq!(text_field(&result.data, "api_key"), Some("sk-azure"));
}

#[test]
fn deployment_model_prefix_supplies_the_provider() {
    let params = DeploymentParams {
        custom_llm_provider: None,
        model: Some("azure/gpt-4".to_owned()),
    };
    let catalog_view = catalog(TestCatalog::default().with_group("gpt-4", params));
    let principal = principal_with(&[
        ("gpt-4", "openai", Some("openai-cred")),
        ("gpt-4", "azure", Some("azure-cred")),
    ]);
    let service = build_service(credential_settings(), catalog_view, team_credentials());
    let result = service.apply(&input(principal, "gpt-4")).unwrap();
    assert_eq!(text_field(&result.data, "api_key"), Some("sk-azure"));
}

#[test]
fn deployment_without_provider_info_leaves_the_first_entry() {
    let params = DeploymentParams {
        custom_llm_provider: None,
        model: Some("gpt-4".to_owned()),
    };
    let catalog_view = catalog(TestCatalog::default().with_group("gpt-4", params));
    let principal = principal_with(&[
        ("gpt-4", "openai", Some("openai-cred")),
        ("gpt-4", "azure", Some("azure-cred")),
    ]);
    let service = build_service(credential_settings(), catalog_view, team_credentials());
    let result = service.apply(&input(principal, "gpt-4")).unwrap();
    assert_eq!(text_field(&result.data, "api_key"), Some("sk-openai"));
}

#[test]
fn deployment_provider_reads_the_pre_alias_group() {
    let catalog_view =
        catalog(TestCatalog::default().with_group("public", deployment("azure", "openai/gpt-4")));
    let principal = OverridesPrincipal {
        team_id: None,
        team_model_aliases: alias_map(&[("public", "resolved")]),
        aliases: alias_map(&[]),
        metadata: None,
        team_metadata: Some(model_config(&[
            ("resolved", "openai", Some("openai-cred")),
            ("resolved", "azure", Some("azure-cred")),
        ])),
        project_metadata: None,
    };
    let service = build_service(credential_settings(), catalog_view, team_credentials());
    let result = service.apply(&input(principal, "public")).unwrap();
    assert_eq!(text_field(&result.data, "model"), Some("resolved"));
    assert_eq!(text_field(&result.data, "api_key"), Some("sk-azure"));
}

#[test]
fn missing_credential_warns_and_copies_nothing() {
    let principal = principal_with(&[("defaultconfig", "azure", Some("ghost"))]);
    let service = build_service(credential_settings(), None, team_credentials());
    let result = service.apply(&input(principal, "gpt-4o")).unwrap();
    assert_eq!(text_field(&result.data, "api_key"), None);
    assert_eq!(
        result.diagnostics,
        vec![OverrideDiagnostic::CredentialNotFound {
            credential: "ghost".to_owned(),
        }]
    );
}

#[test]
fn credential_without_request_fields_applies_nothing() {
    let principal = principal_with(&[("defaultconfig", "azure", Some("team-azure"))]);
    let credentials = vec![credential("team-azure", &[("x-extra", "1")])];
    let service = build_service(credential_settings(), None, credentials);
    let result = service.apply(&input(principal, "gpt-4o")).unwrap();
    assert_eq!(text_field(&result.data, "api_key"), None);
    assert_eq!(
        result.diagnostics,
        vec![OverrideDiagnostic::CredentialApplied {
            credential: "team-azure".to_owned(),
            model: "gpt-4o".to_owned(),
        }]
    );
}

#[rstest]
#[case::provider_map_without_credentials(None)]
#[case::empty_credential_name(Some(""))]
fn entry_without_a_credential_tries_the_default(#[case] name: Option<&str>) {
    let principal = principal_with(&[
        ("gpt-4o", "azure", name),
        ("defaultconfig", "azure", Some("team-azure")),
    ]);
    let service = build_service(credential_settings(), None, team_credentials());
    let result = service.apply(&input(principal, "gpt-4o")).unwrap();
    assert_eq!(text_field(&result.data, "api_key"), Some("sk-team"));
    assert_eq!(
        result.diagnostics,
        vec![
            OverrideDiagnostic::ModelEntryWithoutCredentials {
                model: "gpt-4o".to_owned(),
            },
            OverrideDiagnostic::CredentialApplied {
                credential: "team-azure".to_owned(),
                model: "gpt-4o".to_owned(),
            },
        ]
    );
}

#[test]
fn feature_flag_off_skips_the_walk() {
    let principal = principal_with(&[("defaultconfig", "azure", Some("team-azure"))]);
    let service = build_service(enforced_settings(None, None), None, team_credentials());
    let result = service.apply(&input(principal, "gpt-4o")).unwrap();
    assert_eq!(text_field(&result.data, "api_key"), None);
    assert_eq!(result.diagnostics, Vec::new());
}

#[test]
fn no_model_config_copies_nothing() {
    let principal = principal_with(&[]);
    let service = build_service(credential_settings(), None, team_credentials());
    let result = service.apply(&input(principal, "gpt-4o")).unwrap();
    assert_eq!(text_field(&result.data, "api_key"), None);
    assert_eq!(result.diagnostics, Vec::new());
}

#[test]
fn null_model_config_copies_nothing() {
    let principal = OverridesPrincipal {
        team_id: None,
        team_model_aliases: alias_map(&[]),
        aliases: alias_map(&[]),
        metadata: None,
        team_metadata: Some(JsonObject::from([("model_config".to_owned(), Json::Null)])),
        project_metadata: None,
    };
    let service = build_service(credential_settings(), None, team_credentials());
    let result = service.apply(&input(principal, "gpt-4o")).unwrap();
    assert_eq!(text_field(&result.data, "api_key"), None);
    assert_eq!(result.diagnostics, Vec::new());
}

#[test]
fn an_empty_model_entry_counts_as_absent() {
    let default_credential = JsonObject::from([(
        "litellm_credentials".to_owned(),
        Json::Text("team-azure".to_owned()),
    )]);
    let default_entry = Json::Object(JsonObject::from([(
        "azure".to_owned(),
        Json::Object(default_credential),
    )]));
    let entries = JsonObject::from([
        ("gpt-4o".to_owned(), Json::Object(JsonObject::new())),
        ("defaultconfig".to_owned(), default_entry),
    ]);
    let principal = OverridesPrincipal {
        team_id: None,
        team_model_aliases: alias_map(&[]),
        aliases: alias_map(&[]),
        metadata: None,
        team_metadata: Some(JsonObject::from([(
            "model_config".to_owned(),
            Json::Object(entries),
        )])),
        project_metadata: None,
    };
    let service = build_service(credential_settings(), None, team_credentials());
    let result = service.apply(&input(principal, "gpt-4o")).unwrap();
    assert_eq!(text_field(&result.data, "api_key"), Some("sk-team"));
    assert_eq!(
        result.diagnostics,
        vec![OverrideDiagnostic::CredentialApplied {
            credential: "team-azure".to_owned(),
            model: "gpt-4o".to_owned(),
        }]
    );
}

#[test]
fn client_api_version_survives_and_does_not_gate() {
    let data = request(&[("model", "gpt-4o"), ("api_version", "2024-01-01")]);
    let principal = principal_with(&[("defaultconfig", "azure", Some("team-azure"))]);
    let credentials = vec![credential(
        "team-azure",
        &[("api_key", "sk-team"), ("api_version", "2025-01-01")],
    )];
    let input = RequestModelOverridesInput {
        data,
        principal,
        premium_user: true,
    };
    let result = build_service(credential_settings(), None, credentials)
        .apply(&input)
        .unwrap();
    assert_eq!(text_field(&result.data, "api_key"), Some("sk-team"));
    assert_eq!(text_field(&result.data, "api_version"), Some("2024-01-01"));
}

#[rstest]
#[case::absent(request(&[("user", "someone")]))]
#[case::empty_name(request(&[("model", "")]))]
fn credential_override_skips_a_request_without_a_model(#[case] data: RequestData) {
    let principal = principal_with(&[("defaultconfig", "azure", Some("team-azure"))]);
    let input = RequestModelOverridesInput {
        data,
        principal,
        premium_user: true,
    };
    let result = build_service(credential_settings(), None, team_credentials())
        .apply(&input)
        .unwrap();
    assert_eq!(text_field(&result.data, "api_key"), None);
    assert_eq!(result.diagnostics, Vec::new());
}

#[test]
fn project_model_config_of_the_wrong_type_names_the_field() {
    let principal = OverridesPrincipal {
        team_id: None,
        team_model_aliases: alias_map(&[]),
        aliases: alias_map(&[]),
        metadata: None,
        team_metadata: None,
        project_metadata: Some(JsonObject::from([(
            "model_config".to_owned(),
            Json::Text("gpt-4o".to_owned()),
        )])),
    };
    let service = build_service(credential_settings(), None, team_credentials());
    let error = service.apply(&input(principal, "gpt-4o")).unwrap_err();
    assert_eq!(
        error,
        RequestModelOverridesError::InvalidField {
            field: "project_metadata.model_config".to_owned(),
            expected: "an object",
        }
    );
}

#[test]
fn model_entry_of_the_wrong_type_names_the_field() {
    let providers = Json::Object(JsonObject::from([(
        "gpt-4o".to_owned(),
        Json::Text("azure".to_owned()),
    )]));
    let principal = OverridesPrincipal {
        team_id: None,
        team_model_aliases: alias_map(&[]),
        aliases: alias_map(&[]),
        metadata: None,
        team_metadata: Some(JsonObject::from([("model_config".to_owned(), providers)])),
        project_metadata: None,
    };
    let service = build_service(credential_settings(), None, team_credentials());
    let error = service.apply(&input(principal, "gpt-4o")).unwrap_err();
    assert_eq!(
        error,
        RequestModelOverridesError::InvalidField {
            field: "team_metadata.model_config.gpt-4o".to_owned(),
            expected: "an object",
        }
    );
}

#[test]
fn model_config_of_the_wrong_type_names_the_field() {
    let principal = OverridesPrincipal {
        team_id: None,
        team_model_aliases: alias_map(&[]),
        aliases: alias_map(&[]),
        metadata: None,
        team_metadata: Some(JsonObject::from([(
            "model_config".to_owned(),
            Json::Text("gpt-4o".to_owned()),
        )])),
        project_metadata: None,
    };
    let service = build_service(credential_settings(), None, team_credentials());
    let error = service.apply(&input(principal, "gpt-4o")).unwrap_err();
    assert_eq!(
        error,
        RequestModelOverridesError::InvalidField {
            field: "team_metadata.model_config".to_owned(),
            expected: "an object",
        }
    );
}

#[test]
fn credential_name_of_the_wrong_type_names_its_path() {
    let principal = OverridesPrincipal {
        team_id: None,
        team_model_aliases: alias_map(&[]),
        aliases: alias_map(&[]),
        metadata: None,
        team_metadata: Some(model_config_field(
            "gpt-4o",
            "azure",
            "litellm_credentials",
            Json::Bool(true),
        )),
        project_metadata: None,
    };
    let service = build_service(credential_settings(), None, team_credentials());
    let error = service.apply(&input(principal, "gpt-4o")).unwrap_err();
    assert_eq!(
        error,
        RequestModelOverridesError::InvalidField {
            field: "team_metadata.model_config.gpt-4o.azure.litellm_credentials".to_owned(),
            expected: "a string",
        }
    );
}

#[test]
fn model_that_is_not_text_names_the_field() {
    let data = RequestData::from([("model".to_owned(), Json::Bool(true))]);
    let input = RequestModelOverridesInput {
        data,
        principal: OverridesPrincipal::default(),
        premium_user: true,
    };
    let error = RequestModelOverridesService::default()
        .apply(&input)
        .unwrap_err();
    assert_eq!(
        error,
        RequestModelOverridesError::InvalidField {
            field: "model".to_owned(),
            expected: "a string",
        }
    );
}
