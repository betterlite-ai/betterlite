#![cfg(test)]

mod alias_stale;
mod credential_chain;
mod diagnostics;
mod enforced_extras;
mod support;

use std::collections::BTreeMap;

use betterlite_auth::auth_model_lists::credential::CredentialEntry;
use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::request_model_overrides::Settings as ModelOverridesSettings;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use crate::api::request_destination::RequestData;
use crate::api::request_model_overrides::diagnostics::OverrideDiagnostic;
use crate::api::request_model_overrides::error::{ErrorFacts, RequestModelOverridesError};
use crate::api::request_model_overrides::{
    OverridesPrincipal, RequestModelOverridesInput, RequestModelOverridesResult,
    RequestModelOverridesService,
};

use support::{
    alias_map, build_service, credential, credential_settings, enforced_settings, model_config,
    request, requested, text_field,
};

#[rstest]
#[case::team_then_key(&[("gpt-4o", "team-model")], &[("team-model", "key-model")], "key-model")]
#[case::key_alias_reads_the_team_name(&[("gpt-4o", "team-model")], &[("gpt-4o", "key-model")], "team-model")]
#[case::key_alias_alone(&[], &[("gpt-4o", "key-model")], "key-model")]
#[case::team_alias_alone(&[("gpt-4o", "team-model")], &[], "team-model")]
#[case::no_alias(&[], &[], "gpt-4o")]
#[case::empty_team_target(&[("gpt-4o", "")], &[("", "key-model")], "")]
fn team_then_key_alias(
    #[case] team_aliases: &[(&str, &str)],
    #[case] aliases: &[(&str, &str)],
    #[case] expected: &str,
) {
    let principal = OverridesPrincipal {
        team_id: Some("team-1".to_owned()),
        team_model_aliases: alias_map(team_aliases),
        aliases: alias_map(aliases),
        metadata: None,
        team_metadata: None,
        project_metadata: None,
    };
    let input = RequestModelOverridesInput {
        data: requested("gpt-4o"),
        principal,
        premium_user: true,
    };
    let result = RequestModelOverridesService::default()
        .apply(&input)
        .unwrap();
    assert_eq!(text_field(&result.data, "model"), Some(expected));
    assert_eq!(result.diagnostics, Vec::new());
}

// both alias stages read data.get("model"), so a request without a usable model keeps its data as sent
#[rstest]
#[case::absent(RequestData::new())]
#[case::null(RequestData::from([("model".to_owned(), Json::Null)]))]
#[case::empty(request(&[("model", "")]))]
fn a_request_without_a_model_skips_both_alias_stages(#[case] data: RequestData) {
    let principal = OverridesPrincipal {
        team_id: Some("team-1".to_owned()),
        team_model_aliases: alias_map(&[("gpt-4o", "team-model")]),
        aliases: alias_map(&[("team-model", "key-model")]),
        metadata: None,
        team_metadata: None,
        project_metadata: None,
    };
    let input = RequestModelOverridesInput {
        data: data.clone(),
        principal,
        premium_user: true,
    };
    let result = RequestModelOverridesService::default()
        .apply(&input)
        .unwrap();
    assert_eq!(result.data, data);
}

// the data map holds copied credential values, so the debug line names the redaction and never them
#[test]
fn result_debug_redacts_credential_values() {
    let result = RequestModelOverridesResult {
        data: request(&[("model", "gpt-4o"), ("api_key", "sk-secret")]),
        enforced_params: None,
        diagnostics: Vec::new(),
    };
    let rendered = format!("{result:?}");
    assert!(rendered.contains("<redacted>"));
    assert!(!rendered.contains("sk-secret"));
}

fn chain_credentials() -> Vec<CredentialEntry> {
    vec![
        credential("project-pre", &[("api_key", "sk-project-pre")]),
        credential("project-post", &[("api_key", "sk-project-post")]),
        credential("project-default", &[("api_key", "sk-project-default")]),
        credential("team-pre", &[("api_key", "sk-team-pre")]),
        credential("team-post", &[("api_key", "sk-team-post")]),
        credential("team-default", &[("api_key", "sk-team-default")]),
    ]
}

#[rstest]
#[case::project_pre_alias(
    &[("public", "azure", Some("project-pre"))],
    &[("resolved", "azure", Some("team-post"))],
    "project-pre"
)]
#[case::project_post_alias(
    &[("resolved", "azure", Some("project-post"))],
    &[("resolved", "azure", Some("team-post"))],
    "project-post"
)]
#[case::project_default_config(
    &[("defaultconfig", "azure", Some("project-default"))],
    &[("resolved", "azure", Some("team-post"))],
    "project-default"
)]
#[case::team_pre_alias(&[], &[("public", "azure", Some("team-pre"))], "team-pre")]
#[case::team_post_alias(&[], &[("resolved", "azure", Some("team-post"))], "team-post")]
#[case::team_default_config(&[], &[("defaultconfig", "azure", Some("team-default"))], "team-default")]
fn prealias_credential_override(
    #[case] project: &[(&str, &str, Option<&str>)],
    #[case] team: &[(&str, &str, Option<&str>)],
    #[case] expected: &str,
) {
    let principal = OverridesPrincipal {
        team_id: Some("team-1".to_owned()),
        team_model_aliases: alias_map(&[("public", "resolved")]),
        aliases: alias_map(&[]),
        metadata: None,
        team_metadata: Some(model_config(team)),
        project_metadata: Some(model_config(project)),
    };
    let input = RequestModelOverridesInput {
        data: requested("public"),
        principal,
        premium_user: true,
    };
    let result = build_service(credential_settings(), None, chain_credentials())
        .apply(&input)
        .unwrap();
    assert_eq!(text_field(&result.data, "model"), Some("resolved"));
    let expected_key = format!("sk-{expected}");
    assert_eq!(
        text_field(&result.data, "api_key"),
        Some(expected_key.as_str())
    );
    assert_eq!(
        result.diagnostics,
        vec![OverrideDiagnostic::CredentialApplied {
            credential: expected.to_owned(),
            model: "resolved".to_owned(),
        }]
    );
}

#[derive(Debug)]
enum Expected {
    Passed,
    Missing(&'static str),
    ParentNotObject(&'static str),
    Forbidden,
}

#[derive(Debug, Clone, Copy)]
enum BodyMetadata {
    Absent,
    Keys(&'static [&'static str]),
    Text(&'static str),
    Null,
}

#[rstest]
#[case::present_top_level(&["user"], &[("user", "someone")], BodyMetadata::Absent, true, Expected::Passed)]
#[case::missing_top_level(&["user"], &[], BodyMetadata::Keys(&[]), true, Expected::Missing("user"))]
#[case::missing_nested(
    &["metadata.generation_name"],
    &[],
    BodyMetadata::Keys(&[]),
    true,
    Expected::Missing("metadata.generation_name")
)]
#[case::missing_nested_parent(
    &["metadata.generation_name"],
    &[],
    BodyMetadata::Absent,
    true,
    Expected::Missing("metadata")
)]
#[case::nested_present(
    &["metadata.generation_name"],
    &[],
    BodyMetadata::Keys(&["generation_name"]),
    true,
    Expected::Passed
)]
#[case::parent_not_object(
    &["metadata.generation_name"],
    &[],
    BodyMetadata::Text("value"),
    true,
    Expected::ParentNotObject("metadata")
)]
#[case::parent_null(
    &["metadata.generation_name"],
    &[],
    BodyMetadata::Null,
    true,
    Expected::ParentNotObject("metadata")
)]
#[case::not_premium(&["user"], &[("user", "someone")], BodyMetadata::Absent, false, Expected::Forbidden)]
#[case::empty_list_not_premium(&[], &[], BodyMetadata::Absent, false, Expected::Passed)]
#[case::three_segments_unchecked(
    &["metadata.user.email"],
    &[],
    BodyMetadata::Keys(&[]),
    true,
    Expected::Passed
)]
fn enforced_params_missing(
    #[case] enforced: &[&str],
    #[case] body: &[(&str, &str)],
    #[case] metadata: BodyMetadata,
    #[case] premium_user: bool,
    #[case] expected: Expected,
) {
    let mut data = request(body);
    fill_metadata(&mut data, metadata);
    let input = RequestModelOverridesInput {
        data,
        principal: OverridesPrincipal::default(),
        premium_user,
    };
    let routed = build_service(enforced_settings(Some(enforced), None), None, Vec::new());
    assert_enforced_outcome(routed.apply(&input), expected, enforced);
}

fn fill_metadata(data: &mut RequestData, metadata: BodyMetadata) {
    match metadata {
        BodyMetadata::Absent => {}
        BodyMetadata::Keys(keys) => {
            let bucket: JsonObject = keys
                .iter()
                .map(|key| ((*key).to_owned(), Json::Text("value".to_owned())))
                .collect();
            data.insert("metadata".to_owned(), Json::Object(bucket));
        }
        BodyMetadata::Text(text) => {
            data.insert("metadata".to_owned(), Json::Text(text.to_owned()));
        }
        BodyMetadata::Null => {
            data.insert("metadata".to_owned(), Json::Null);
        }
    }
}

fn assert_enforced_outcome(
    result: Result<RequestModelOverridesResult, RequestModelOverridesError>,
    expected: Expected,
    enforced: &[&str],
) {
    match (result, expected) {
        (Ok(result), Expected::Passed) => assert_eq!(
            result.enforced_params,
            Some(
                enforced
                    .iter()
                    .map(|item| (*item).to_owned())
                    .collect::<Vec<String>>()
            )
        ),
        (Err(error), Expected::Missing(param)) => {
            assert_eq!(
                error,
                RequestModelOverridesError::MissingEnforcedParam {
                    param: param.to_owned()
                }
            );
            assert_eq!(error.facts().param.as_deref(), Some(param));
            assert_eq!(error.facts().status, 400);
        }
        (Err(error), Expected::ParentNotObject(field)) => {
            assert!(matches!(
                error,
                RequestModelOverridesError::EnforcedParamParentNotObject { .. }
            ));
            assert_eq!(error.facts().param.as_deref(), Some(field));
        }
        (Err(error), Expected::Forbidden) => {
            assert_eq!(
                error.facts(),
                ErrorFacts {
                    status: 403,
                    error_type: "permission_error",
                    param: None,
                    code: "403",
                }
            );
        }
        (other, want) => panic!("{want:?} expected, got {other:?}"),
    }
}

fn load_settings(
    source: &str,
    env: &[(&str, &str)],
) -> Result<ModelOverridesSettings, ConfigError> {
    let lookup: BTreeMap<String, String> = env
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load(source)?;
    Ok(config.sections.request_model_overrides)
}

#[derive(Debug)]
enum Wiring {
    Credential(Option<&'static str>),
    Enforced(Option<Vec<String>>),
}

#[rstest]
#[case::credential_overrides_on(
    "litellm_settings:\n  enable_model_config_credential_overrides: true\n",
    Wiring::Credential(Some("sk-team"))
)]
#[case::credential_overrides_unset("", Wiring::Credential(None))]
#[case::credential_overrides_off(
    "litellm_settings:\n  enable_model_config_credential_overrides: false\n",
    Wiring::Credential(None)
)]
#[case::enforced_params_listed(
    "general_settings:\n  enforced_params: [user]\n",
    Wiring::Enforced(Some(vec!["user".to_owned()]))
)]
#[case::enforced_params_unset("", Wiring::Enforced(None))]
fn loads_settings(#[case] source: &str, #[case] expected: Wiring) {
    let settings = load_settings(source, &[]).unwrap();
    let principal = OverridesPrincipal {
        team_id: None,
        team_model_aliases: alias_map(&[]),
        aliases: alias_map(&[]),
        metadata: None,
        team_metadata: Some(model_config(&[(
            "defaultconfig",
            "azure",
            Some("team-azure"),
        )])),
        project_metadata: None,
    };
    let input = RequestModelOverridesInput {
        data: request(&[("model", "gpt-4o"), ("user", "someone")]),
        principal,
        premium_user: true,
    };
    let result = build_service(
        settings,
        None,
        vec![credential("team-azure", &[("api_key", "sk-team")])],
    )
    .apply(&input)
    .unwrap();
    match expected {
        Wiring::Credential(api_key) => assert_eq!(text_field(&result.data, "api_key"), api_key),
        Wiring::Enforced(enforced) => assert_eq!(result.enforced_params, enforced),
    }
}
