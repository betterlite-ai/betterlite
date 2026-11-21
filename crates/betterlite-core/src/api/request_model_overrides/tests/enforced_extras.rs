use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::support::{alias_map, build_service, enforced_settings, request};
use crate::api::request_model_overrides::error::RequestModelOverridesError;
use crate::api::request_model_overrides::{OverridesPrincipal, RequestModelOverridesInput};

fn principal(metadata: JsonObject) -> OverridesPrincipal {
    OverridesPrincipal {
        team_id: None,
        team_model_aliases: alias_map(&[]),
        aliases: alias_map(&[]),
        metadata: Some(metadata),
        team_metadata: None,
        project_metadata: None,
    }
}

fn metadata(entries: &[(&str, Json)]) -> JsonObject {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn service_account_id() -> Json {
    Json::Text("sa-1".to_owned())
}

#[test]
fn service_account_params_reach_service_accounts_only() {
    let service = build_service(
        enforced_settings(Some(&["user"]), Some(&["metadata.service_account_id"])),
        None,
        Vec::new(),
    );
    let body = request(&[("model", "gpt-4o"), ("user", "someone")]);
    let ordinary = RequestModelOverridesInput {
        data: body.clone(),
        principal: principal(metadata(&[])),
        premium_user: true,
    };
    assert_eq!(
        service.apply(&ordinary).unwrap().enforced_params,
        Some(vec!["user".to_owned()])
    );
    let account = RequestModelOverridesInput {
        data: body,
        principal: principal(metadata(&[("service_account_id", service_account_id())])),
        premium_user: true,
    };
    assert_eq!(
        service.apply(&account).unwrap_err(),
        RequestModelOverridesError::MissingEnforcedParam {
            param: "metadata".to_owned()
        }
    );
}

#[test]
fn key_metadata_params_extend_the_general_list() {
    let key_enforced = Json::List(vec![Json::Text("metadata.generation_name".to_owned())]);
    let input = RequestModelOverridesInput {
        data: request(&[("model", "gpt-4o"), ("user", "someone")]),
        principal: principal(metadata(&[("enforced_params", key_enforced)])),
        premium_user: true,
    };
    let error = build_service(enforced_settings(Some(&["user"]), None), None, Vec::new())
        .apply(&input)
        .unwrap_err();
    assert_eq!(
        error,
        RequestModelOverridesError::MissingEnforcedParam {
            param: "metadata".to_owned()
        }
    );
}

#[test]
fn enforced_params_keep_the_general_service_and_key_order() {
    let key_enforced = Json::List(vec![Json::Text("c".to_owned())]);
    let input = RequestModelOverridesInput {
        data: request(&[("model", "gpt-4o"), ("a", "1"), ("b", "2"), ("c", "3")]),
        principal: principal(metadata(&[
            ("service_account_id", service_account_id()),
            ("enforced_params", key_enforced),
        ])),
        premium_user: true,
    };
    let result = build_service(
        enforced_settings(Some(&["a"]), Some(&["b"])),
        None,
        Vec::new(),
    )
    .apply(&input)
    .unwrap();
    assert_eq!(
        result.enforced_params,
        Some(vec!["a".to_owned(), "b".to_owned(), "c".to_owned()])
    );
}

#[test]
fn null_key_metadata_params_are_ignored() {
    let input = RequestModelOverridesInput {
        data: request(&[("model", "gpt-4o")]),
        principal: principal(metadata(&[("enforced_params", Json::Null)])),
        premium_user: false,
    };
    let result = build_service(enforced_settings(None, None), None, Vec::new())
        .apply(&input)
        .unwrap();
    assert_eq!(result.enforced_params, None);
}

#[test]
fn empty_key_metadata_params_still_pin_an_empty_list() {
    let input = RequestModelOverridesInput {
        data: request(&[("model", "gpt-4o")]),
        principal: principal(metadata(&[("enforced_params", Json::List(Vec::new()))])),
        premium_user: false,
    };
    let result = build_service(enforced_settings(None, None), None, Vec::new())
        .apply(&input)
        .unwrap();
    assert_eq!(result.enforced_params, Some(Vec::new()));
}

#[rstest]
#[case::text(Json::Text("user".to_owned()))]
#[case::list_with_a_boolean(Json::List(vec![Json::Text("user".to_owned()), Json::Bool(true)]))]
fn invalid_key_metadata_params_name_the_field(#[case] value: Json) {
    let input = RequestModelOverridesInput {
        data: request(&[("model", "gpt-4o")]),
        principal: principal(metadata(&[("enforced_params", value)])),
        premium_user: true,
    };
    let error = build_service(enforced_settings(None, None), None, Vec::new())
        .apply(&input)
        .unwrap_err();
    assert_eq!(
        error,
        RequestModelOverridesError::InvalidField {
            field: "metadata.enforced_params".to_owned(),
            expected: "a list of strings",
        }
    );
}
